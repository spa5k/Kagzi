use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{PgPool, QueryBuilder};
use tracing::instrument;
use uuid::Uuid;

use crate::error::StoreError;
use crate::models::{
    CreateSchedule, ListSchedulesParams, PaginatedResult, Schedule, ScheduleCursor, UpdateSchedule,
    clamp_max_catchup,
};
use crate::postgres::columns;
use crate::postgres::query::{FilterBuilder, push_limit, push_tuple_cursor};
use crate::repository::WorkflowScheduleRepository;

#[derive(sqlx::FromRow)]
struct ScheduleRow {
    schedule_id: Uuid,
    namespace_id: String,
    task_queue: String,
    workflow_type: String,
    cron_expr: String,
    input: serde_json::Value,
    context: Option<serde_json::Value>,
    enabled: bool,
    max_catchup: i32,
    next_fire_at: DateTime<Utc>,
    last_fired_at: Option<DateTime<Utc>>,
    version: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl ScheduleRow {
    fn into_model(self) -> Schedule {
        Schedule {
            schedule_id: self.schedule_id,
            namespace_id: self.namespace_id,
            task_queue: self.task_queue,
            workflow_type: self.workflow_type,
            cron_expr: self.cron_expr,
            input: self.input,
            context: self.context,
            enabled: self.enabled,
            max_catchup: self.max_catchup,
            next_fire_at: self.next_fire_at,
            last_fired_at: self.last_fired_at,
            version: self.version,
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

#[derive(Clone)]
pub struct PgScheduleRepository {
    pool: PgPool,
}

impl PgScheduleRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl WorkflowScheduleRepository for PgScheduleRepository {
    #[instrument(skip(self, params))]
    async fn create(&self, params: CreateSchedule) -> Result<Uuid, StoreError> {
        let max_catchup = clamp_max_catchup(params.max_catchup);

        let schedule_id: Uuid = sqlx::query_scalar(
            r#"
            INSERT INTO kagzi.schedules (
                namespace_id, task_queue, workflow_type, cron_expr, input, context,
                enabled, max_catchup, next_fire_at, last_fired_at, version
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, NULL, $10)
            RETURNING schedule_id
            "#,
        )
        .bind(params.namespace_id)
        .bind(params.task_queue)
        .bind(params.workflow_type)
        .bind(params.cron_expr)
        .bind(params.input)
        .bind(params.context)
        .bind(params.enabled)
        .bind(max_catchup)
        .bind(params.next_fire_at)
        .bind(params.version)
        .fetch_one(&self.pool)
        .await?;

        Ok(schedule_id)
    }

    #[instrument(skip(self))]
    async fn find_by_id(
        &self,
        id: Uuid,
        namespace_id: &str,
    ) -> Result<Option<Schedule>, StoreError> {
        let row = sqlx::query_as::<_, ScheduleRow>(
            r#"
            SELECT schedule_id, namespace_id, task_queue, workflow_type, cron_expr,
                   input, context, enabled, max_catchup, next_fire_at, last_fired_at,
                   version, created_at, updated_at
            FROM kagzi.schedules
            WHERE schedule_id = $1 AND namespace_id = $2
            "#,
        )
        .bind(id)
        .bind(namespace_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| r.into_model()))
    }

    #[instrument(skip(self, params))]
    async fn list(
        &self,
        params: ListSchedulesParams,
    ) -> Result<PaginatedResult<Schedule, ScheduleCursor>, StoreError> {
        let page_size = params.page_size.max(1) as usize;
        let limit = (page_size + 1) as i64;

        let mut filters = FilterBuilder::select(columns::schedule::BASE, "kagzi.schedules");
        filters.and_eq("namespace_id", &params.namespace_id);
        filters.and_optional_eq("task_queue", params.task_queue.as_deref());

        if let Some(ref cursor) = params.cursor {
            push_tuple_cursor(
                filters.builder(),
                &["created_at", "schedule_id"],
                "<",
                |b: &mut QueryBuilder<'_, _>| {
                    b.push_bind(cursor.created_at)
                        .push(", ")
                        .push_bind(cursor.schedule_id);
                },
            );
        }

        let mut builder = filters.finalize();
        builder.push(" ORDER BY created_at DESC, schedule_id DESC");
        push_limit(&mut builder, limit);

        let rows: Vec<ScheduleRow> = builder.build_query_as().fetch_all(&self.pool).await?;

        let has_more = rows.len() > page_size;
        let items: Vec<Schedule> = rows
            .into_iter()
            .take(page_size)
            .map(|r| r.into_model())
            .collect();

        let next_cursor = if has_more {
            items.last().map(|s| ScheduleCursor {
                created_at: s.created_at,
                schedule_id: s.schedule_id,
            })
        } else {
            None
        };

        Ok(PaginatedResult {
            items,
            next_cursor,
            has_more,
        })
    }

    #[instrument(skip(self, params))]
    async fn update(
        &self,
        id: Uuid,
        namespace_id: &str,
        params: UpdateSchedule,
    ) -> Result<(), StoreError> {
        let max_catchup = params.max_catchup.map(clamp_max_catchup);

        sqlx::query(
            r#"
            UPDATE kagzi.schedules
            SET task_queue = COALESCE($3, task_queue),
                workflow_type = COALESCE($4, workflow_type),
                cron_expr = COALESCE($5, cron_expr),
                input = COALESCE($6, input),
                context = COALESCE($7, context),
                enabled = COALESCE($8, enabled),
                max_catchup = COALESCE($9, max_catchup),
                next_fire_at = COALESCE($10, next_fire_at),
                version = COALESCE($11, version),
                updated_at = NOW()
            WHERE schedule_id = $1 AND namespace_id = $2
            "#,
        )
        .bind(id)
        .bind(namespace_id)
        .bind(params.task_queue)
        .bind(params.workflow_type)
        .bind(params.cron_expr)
        .bind(params.input)
        .bind(params.context)
        .bind(params.enabled)
        .bind(max_catchup)
        .bind(params.next_fire_at)
        .bind(params.version)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    #[instrument(skip(self))]
    async fn delete(&self, id: Uuid, namespace_id: &str) -> Result<bool, StoreError> {
        let result = sqlx::query(
            r#"
            DELETE FROM kagzi.schedules
            WHERE schedule_id = $1 AND namespace_id = $2
            "#,
        )
        .bind(id)
        .bind(namespace_id)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }

    #[instrument(skip(self))]
    async fn due_schedules(
        &self,
        now: DateTime<Utc>,
        limit: i32,
    ) -> Result<Vec<Schedule>, StoreError> {
        let rows = sqlx::query_as::<_, ScheduleRow>(
            r#"
            WITH due AS (
                SELECT schedule_id, namespace_id, task_queue, workflow_type, cron_expr,
                       input, context, enabled, max_catchup, next_fire_at, last_fired_at,
                       version, created_at, updated_at
                FROM kagzi.schedules
                WHERE enabled = TRUE
                  AND next_fire_at <= $1
                ORDER BY next_fire_at ASC
                LIMIT $2
                FOR UPDATE SKIP LOCKED
            )
            SELECT * FROM due
            "#,
        )
        .bind(now)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(|r| r.into_model()).collect())
    }

    #[instrument(skip(self))]
    async fn advance_schedule(
        &self,
        id: Uuid,
        last_fired: DateTime<Utc>,
        next_fire: DateTime<Utc>,
    ) -> Result<(), StoreError> {
        sqlx::query(
            r#"
            UPDATE kagzi.schedules
            SET last_fired_at = $2,
                next_fire_at = $3,
                updated_at = NOW()
            WHERE schedule_id = $1
            "#,
        )
        .bind(id)
        .bind(last_fired)
        .bind(next_fire)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    #[instrument(skip(self))]
    async fn record_firing(
        &self,
        schedule_id: Uuid,
        fire_at: DateTime<Utc>,
        run_id: Uuid,
    ) -> Result<(), StoreError> {
        sqlx::query(
            r#"
            INSERT INTO kagzi.schedule_firings (schedule_id, fire_at, run_id)
            VALUES ($1, $2, $3)
            ON CONFLICT (schedule_id, fire_at) DO NOTHING
            "#,
        )
        .bind(schedule_id)
        .bind(fire_at)
        .bind(run_id)
        .execute(&self.pool)
        .await?;

        Ok(())
    }
}
