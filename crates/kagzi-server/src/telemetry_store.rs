use kagzi_store::PgStore;
use sqlx::{QueryBuilder, Row};
use tracing::{info, warn};
use uuid::Uuid;

use crate::queue_store::ensure_task_queue_exists;

async fn ensure_queue_best_effort(store: &PgStore, namespace: &str, task_queue: &str) {
    let _ = ensure_task_queue_exists(store, namespace, task_queue).await;
}

pub async fn record_server_event(
    store: &PgStore,
    enabled: bool,
    level: &str,
    event_type: &str,
    message: &str,
    namespace: Option<&str>,
    extra: serde_json::Value,
) {
    if !enabled {
        return;
    }

    let ns = namespace.unwrap_or("");
    if let Err(e) = sqlx::query(
        r#"
        INSERT INTO kagzi.server_telemetry_events (namespace, level, event_type, message, extra)
        VALUES ($1, $2, $3, $4, $5)
        "#,
    )
    .bind(ns)
    .bind(level)
    .bind(event_type)
    .bind(message)
    .bind(extra)
    .execute(store.pool())
    .await
    {
        warn!(error = ?e, "Failed to persist server telemetry event");
    }
}

pub async fn record_queue_publish_result(
    store: &PgStore,
    enabled: bool,
    namespace: &str,
    task_queue: &str,
    ok: bool,
    error_message: Option<&str>,
) {
    if !enabled {
        return;
    }

    ensure_queue_best_effort(store, namespace, task_queue).await;

    let error_message = error_message.unwrap_or("");
    let publish_errors_inc: i64 = if ok { 0 } else { 1 };
    let now = chrono::Utc::now();
    let last_ok_at = ok.then_some(now);
    let last_err_at = (!ok).then_some(now);

    // Best-effort; do not fail correctness path.
    let _ = sqlx::query(
        r#"
        INSERT INTO kagzi.queue_telemetry_state (
            namespace, task_queue,
            publish_attempts, publish_errors,
            last_publish_ok_at, last_publish_error_at, last_publish_error,
            updated_at
        )
        VALUES ($1, $2, 1, $3, $4, $5, $6, NOW())
        ON CONFLICT (namespace, task_queue) DO UPDATE SET
            publish_attempts = kagzi.queue_telemetry_state.publish_attempts + 1,
            publish_errors = kagzi.queue_telemetry_state.publish_errors + $3,
            last_publish_ok_at = COALESCE($4, kagzi.queue_telemetry_state.last_publish_ok_at),
            last_publish_error_at = COALESCE($5, kagzi.queue_telemetry_state.last_publish_error_at),
            last_publish_error = CASE WHEN $3 > 0 THEN $6 ELSE kagzi.queue_telemetry_state.last_publish_error END,
            updated_at = NOW()
        "#,
    )
    .bind(namespace)
    .bind(task_queue)
    .bind(publish_errors_inc)
    .bind(last_ok_at)
    .bind(last_err_at)
    .bind(error_message)
    .execute(store.pool())
    .await;
}

pub async fn touch_queue_due_work_notified(
    store: &PgStore,
    enabled: bool,
    namespace: &str,
    task_queue: &str,
) {
    if !enabled {
        return;
    }
    ensure_queue_best_effort(store, namespace, task_queue).await;
    let _ = sqlx::query(
        r#"
        INSERT INTO kagzi.queue_telemetry_state (namespace, task_queue, updated_at, last_due_work_notified_at)
        VALUES ($1, $2, NOW(), NOW())
        ON CONFLICT (namespace, task_queue) DO UPDATE SET
            updated_at = NOW(),
            last_due_work_notified_at = NOW()
        "#,
    )
    .bind(namespace)
    .bind(task_queue)
    .execute(store.pool())
    .await;
}

pub async fn upsert_queue_depths(
    store: &PgStore,
    enabled: bool,
    namespace: &str,
    rows: Vec<QueueDepthRow>,
    last_due_work_notified_at: Option<chrono::DateTime<chrono::Utc>>,
) -> Result<(), kagzi_store::StoreError> {
    if !enabled {
        return Ok(());
    }
    if rows.is_empty() {
        return Ok(());
    }

    for r in &rows {
        ensure_queue_best_effort(store, namespace, &r.task_queue).await;
    }

    let notified_at = last_due_work_notified_at;

    let mut builder = QueryBuilder::new(
        r#"
        INSERT INTO kagzi.queue_telemetry_state (
            namespace, task_queue, updated_at,
            pending_count, sleeping_count, running_count, due_count,
            last_due_work_notified_at
        )
        "#,
    );
    builder.push_values(rows.iter(), |mut b, r| {
        b.push_bind(namespace)
            .push_bind(&r.task_queue)
            .push("NOW()")
            .push_bind(r.pending_count)
            .push_bind(r.sleeping_count)
            .push_bind(r.running_count)
            .push_bind(r.due_count)
            .push_bind(notified_at);
    });

    builder.push(
        r#"
        ON CONFLICT (namespace, task_queue) DO UPDATE SET
            updated_at = NOW(),
            pending_count = EXCLUDED.pending_count,
            sleeping_count = EXCLUDED.sleeping_count,
            running_count = EXCLUDED.running_count,
            due_count = EXCLUDED.due_count,
            last_due_work_notified_at = COALESCE(EXCLUDED.last_due_work_notified_at, kagzi.queue_telemetry_state.last_due_work_notified_at)
        "#,
    );

    builder.build().execute(store.pool()).await?;
    Ok(())
}

#[derive(Debug, Clone)]
pub struct QueueDepthRow {
    pub task_queue: String,
    pub pending_count: i64,
    pub sleeping_count: i64,
    pub running_count: i64,
    pub due_count: i64,
}

pub async fn refresh_worker_active_counts(
    store: &PgStore,
    enabled: bool,
) -> Result<(), kagzi_store::StoreError> {
    if !enabled {
        return Ok(());
    }

    // Compute authoritative active counts from current RUNNING leases.
    // Keep it best-effort and cheap; UI uses telemetry_state rows if present.
    let rows = sqlx::query(
        r#"
        SELECT locked_by, COUNT(*)::BIGINT as count
        FROM kagzi.workflow_runs
        WHERE status = 'RUNNING'
          AND available_at > NOW()
          AND locked_by IS NOT NULL
        GROUP BY locked_by
        "#,
    )
    .fetch_all(store.pool())
    .await?;

    // Reset to 0 then set for workers with active leases.
    // This keeps counts consistent even when a worker goes idle.
    let _ =
        sqlx::query("UPDATE kagzi.worker_telemetry_state SET active_workflows_authoritative = 0")
            .execute(store.pool())
            .await?;

    if rows.is_empty() {
        return Ok(());
    }

    let values: Vec<(Uuid, i64)> = rows
        .iter()
        .filter_map(|r| {
            let locked_by: String = r.try_get("locked_by").ok()?;
            let count: i64 = r.try_get::<i64, _>("count").ok().unwrap_or(0);
            let worker_id = Uuid::parse_str(&locked_by).ok()?;
            Some((worker_id, count))
        })
        .collect();

    if values.is_empty() {
        return Ok(());
    }

    let mut builder = QueryBuilder::new(
        "UPDATE kagzi.worker_telemetry_state AS s SET active_workflows_authoritative = v.count FROM (VALUES ",
    );
    builder.push_values(values.iter(), |mut b, (worker_id, count)| {
        b.push_bind(worker_id).push_bind(count);
    });
    builder.push(") AS v(worker_id, count) WHERE s.worker_id = v.worker_id");
    builder.build().execute(store.pool()).await?;

    info!(
        workers = values.len(),
        "Refreshed authoritative worker active counts"
    );
    Ok(())
}

pub async fn refresh_queue_depths(
    store: &PgStore,
    enabled: bool,
    limit: i64,
) -> Result<(), kagzi_store::StoreError> {
    if !enabled {
        return Ok(());
    }

    let limit = limit.clamp(1, 10_000);
    let rows = sqlx::query(
        r#"
        SELECT
            namespace,
            task_queue,
            COUNT(*) FILTER (WHERE status = 'PENDING')::BIGINT AS pending_count,
            COUNT(*) FILTER (WHERE status = 'SLEEPING')::BIGINT AS sleeping_count,
            COUNT(*) FILTER (WHERE status = 'RUNNING')::BIGINT AS running_count,
            COUNT(*) FILTER (
                WHERE status IN ('PENDING', 'SLEEPING') AND available_at <= NOW()
            )::BIGINT AS due_count
        FROM kagzi.workflow_runs
        WHERE status IN ('PENDING', 'SLEEPING', 'RUNNING')
        GROUP BY namespace, task_queue
        ORDER BY due_count DESC, pending_count DESC
        LIMIT $1
        "#,
    )
    .bind(limit)
    .fetch_all(store.pool())
    .await?;

    if rows.is_empty() {
        return Ok(());
    }

    let mut by_ns: std::collections::HashMap<String, Vec<QueueDepthRow>> =
        std::collections::HashMap::new();
    for r in rows {
        let namespace: String = r.try_get("namespace").unwrap_or_default();
        let task_queue: String = r.try_get("task_queue").unwrap_or_default();
        let pending_count: i64 = r.try_get::<i64, _>("pending_count").unwrap_or(0);
        let sleeping_count: i64 = r.try_get::<i64, _>("sleeping_count").unwrap_or(0);
        let running_count: i64 = r.try_get::<i64, _>("running_count").unwrap_or(0);
        let due_count: i64 = r.try_get::<i64, _>("due_count").unwrap_or(0);

        by_ns.entry(namespace).or_default().push(QueueDepthRow {
            task_queue,
            pending_count,
            sleeping_count,
            running_count,
            due_count,
        });
    }

    for (ns, rows) in by_ns {
        upsert_queue_depths(store, enabled, &ns, rows, None).await?;
    }

    Ok(())
}
