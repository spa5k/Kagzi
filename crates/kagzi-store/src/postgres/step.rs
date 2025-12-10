use async_trait::async_trait;
use chrono::Utc;
use sqlx::{PgPool, Postgres, QueryBuilder, Transaction};
use tracing::{instrument, warn};
use uuid::Uuid;

use crate::error::StoreError;
use crate::models::{
    BeginStepParams, BeginStepResult, FailStepParams, FailStepResult, ListStepsParams,
    PaginatedResult, RetryPolicy, RetryTriggered, StepCursor, StepKind, StepRetryInfo, StepRun,
};
use crate::repository::StepRepository;

const STEP_COLUMNS: &str = "\
    attempt_id, run_id, step_id, namespace_id, step_kind, attempt_number, status, \
    input, output, error, child_workflow_run_id, created_at, started_at, \
    finished_at, retry_at, retry_policy";

#[derive(sqlx::FromRow)]
struct StepRunRow {
    attempt_id: Uuid,
    run_id: Uuid,
    step_id: String,
    namespace_id: String,
    step_kind: String,
    attempt_number: i32,
    status: String,
    input: Option<serde_json::Value>,
    output: Option<serde_json::Value>,
    error: Option<String>,
    child_workflow_run_id: Option<Uuid>,
    created_at: Option<chrono::DateTime<chrono::Utc>>,
    started_at: Option<chrono::DateTime<chrono::Utc>>,
    finished_at: Option<chrono::DateTime<chrono::Utc>>,
    retry_at: Option<chrono::DateTime<chrono::Utc>>,
    retry_policy: Option<serde_json::Value>,
}

struct StepResultInsert<'a> {
    run_id: Uuid,
    step_id: &'a str,
    step_kind: StepKind,
    status: &'a str,
    output: Option<&'a serde_json::Value>,
    error: Option<&'a str>,
}

impl StepRunRow {
    fn into_model(self) -> Result<StepRun, StoreError> {
        let step_kind = self.step_kind.parse().map_err(|_| {
            StoreError::invalid_state(format!("invalid step kind: {}", self.step_kind))
        })?;
        let status = self.status.parse().map_err(|_| {
            StoreError::invalid_state(format!("invalid step status: {}", self.status))
        })?;

        Ok(StepRun {
            attempt_id: self.attempt_id,
            run_id: self.run_id,
            step_id: self.step_id,
            namespace_id: self.namespace_id,
            step_kind,
            attempt_number: self.attempt_number,
            status,
            input: self.input,
            output: self.output,
            error: self.error,
            child_workflow_run_id: self.child_workflow_run_id,
            created_at: self.created_at,
            started_at: self.started_at,
            finished_at: self.finished_at,
            retry_at: self.retry_at,
            retry_policy: self
                .retry_policy
                .and_then(|v| serde_json::from_value(v).ok()),
        })
    }
}

#[derive(sqlx::FromRow)]
struct RetryInfoRow {
    attempt_number: i32,
    policy: Option<serde_json::Value>,
}

#[derive(sqlx::FromRow)]
struct ExistingStepRow {
    status: String,
    output: Option<serde_json::Value>,
    retry_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(sqlx::FromRow)]
struct RetryTriggeredRow {
    run_id: Uuid,
    step_id: String,
    attempt_number: i32,
}

#[derive(Clone)]
pub struct PgStepRepository {
    pool: PgPool,
}

impl PgStepRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    async fn insert_step_result(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        params: StepResultInsert<'_>,
    ) -> Result<(), StoreError> {
        let attempt_id = Uuid::now_v7();
        sqlx::query!(
            r#"
            INSERT INTO kagzi.step_runs (attempt_id, run_id, step_id, step_kind, status, output, error, finished_at, is_latest, attempt_number, namespace_id)
            VALUES ($1, $2, $3, $4, $5, $6, $7, NOW(), true, 
                    COALESCE((SELECT MAX(attempt_number) FROM kagzi.step_runs WHERE run_id = $2 AND step_id = $3), 0) + 1,
                    (SELECT namespace_id FROM kagzi.workflow_runs WHERE run_id = $2))
            "#,
            attempt_id,
            params.run_id,
            params.step_id,
            params.step_kind.as_ref(),
            params.status,
            params.output,
            params.error
        )
        .execute(&mut **tx)
        .await?;

        Ok(())
    }

    async fn get_retry_info(
        &self,
        run_id: Uuid,
        step_id: &str,
    ) -> Result<Option<StepRetryInfo>, StoreError> {
        let row: Option<RetryInfoRow> = sqlx::query_as!(
            RetryInfoRow,
            r#"
            SELECT attempt_number, COALESCE(sr.retry_policy, wr.retry_policy) as policy
            FROM kagzi.step_runs sr
            JOIN kagzi.workflow_runs wr ON sr.run_id = wr.run_id
            WHERE sr.run_id = $1 AND sr.step_id = $2 AND sr.is_latest = true
            "#,
            run_id,
            step_id
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| StepRetryInfo {
            attempt_number: r.attempt_number,
            retry_policy: r
                .policy
                .and_then(|v| serde_json::from_value::<RetryPolicy>(v).ok()),
        }))
    }

    async fn latest_step_kind_strict(
        &self,
        run_id: Uuid,
        step_id: &str,
    ) -> Result<StepKind, StoreError> {
        self.latest_step_kind_impl(run_id, step_id, true).await
    }

    async fn latest_step_kind_impl(
        &self,
        run_id: Uuid,
        step_id: &str,
        strict: bool,
    ) -> Result<StepKind, StoreError> {
        let kind = sqlx::query_scalar!(
            r#"
                SELECT step_kind
                FROM kagzi.step_runs
                WHERE run_id = $1 AND step_id = $2 AND is_latest = true
            "#,
            run_id,
            step_id
        )
        .fetch_optional(&self.pool)
        .await?;

        match kind {
            Some(s) => Ok(match s.as_str() {
                "SLEEP" => StepKind::Sleep,
                _ => StepKind::Function,
            }),
            None => {
                if strict {
                    Err(StoreError::not_found(
                        "step_run",
                        format!("{}:{}", run_id, step_id),
                    ))
                } else {
                    warn!(
                        run_id = %run_id,
                        step_id = %step_id,
                        "Step not found, defaulting to Function kind"
                    );
                    Ok(StepKind::Function)
                }
            }
        }
    }
}

#[async_trait]
impl StepRepository for PgStepRepository {
    #[instrument(skip(self))]
    async fn find_by_id(&self, attempt_id: Uuid) -> Result<Option<StepRun>, StoreError> {
        let row = sqlx::query_as!(
            StepRunRow,
            r#"
            SELECT
                attempt_id as "attempt_id!",
                run_id as "run_id!",
                step_id as "step_id!",
                namespace_id as "namespace_id!",
                step_kind,
                attempt_number,
                status,
                input,
                output,
                error,
                child_workflow_run_id,
                created_at,
                started_at,
                finished_at,
                retry_at,
                retry_policy
            FROM kagzi.step_runs
            WHERE attempt_id = $1
            "#,
            attempt_id
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| r.into_model()).transpose()?)
    }

    #[instrument(skip(self, params))]
    async fn list(
        &self,
        params: ListStepsParams,
    ) -> Result<PaginatedResult<StepRun, StepCursor>, StoreError> {
        let page_size = params.page_size.max(1) as usize;
        let limit = (page_size + 1) as i64;

        let mut builder = QueryBuilder::new("SELECT ");
        builder.push(STEP_COLUMNS);
        builder.push(" FROM kagzi.step_runs WHERE run_id = ");
        builder.push_bind(params.run_id);
        builder.push(" AND namespace_id = ");
        builder.push_bind(&params.namespace_id);
        if let Some(step_id) = &params.step_id {
            builder.push(" AND step_id = ").push_bind(step_id);
        }

        if let Some(ref cursor) = params.cursor {
            builder.push(" AND (created_at, attempt_id) > (");
            builder.push_bind(cursor.created_at);
            builder.push(", ");
            builder.push_bind(cursor.attempt_id);
            builder.push(")");
        }

        builder.push(" ORDER BY created_at ASC, attempt_id ASC LIMIT ");
        builder.push_bind(limit);

        let rows: Vec<StepRunRow> = builder.build_query_as().fetch_all(&self.pool).await?;

        let has_more = rows.len() > page_size;
        let items: Vec<StepRun> = rows
            .into_iter()
            .take(page_size)
            .map(|r| r.into_model())
            .collect::<Result<_, _>>()?;

        let next_cursor = if has_more {
            items.last().and_then(|s| {
                s.created_at.map(|created_at| StepCursor {
                    created_at,
                    attempt_id: s.attempt_id,
                })
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
    async fn begin(&self, params: BeginStepParams) -> Result<BeginStepResult, StoreError> {
        let mut tx = self.pool.begin().await?;

        let existing: Option<ExistingStepRow> = sqlx::query_as!(
            ExistingStepRow,
            r#"
            SELECT status, output, retry_at
            FROM kagzi.step_runs
            WHERE run_id = $1 AND step_id = $2 AND is_latest = true
            "#,
            params.run_id,
            params.step_id
        )
        .fetch_optional(&mut *tx)
        .await?;

        if let Some(ref step) = existing {
            if step.status == "COMPLETED" {
                return Ok(BeginStepResult {
                    should_execute: false,
                    cached_output: step.output.clone(),
                });
            }

            if step.status == "PENDING"
                && let Some(retry_at) = step.retry_at
                && retry_at > Utc::now()
            {
                return Err(StoreError::precondition_failed(format!(
                    "Step is in backoff. Retry scheduled at {}",
                    retry_at
                )));
            }
        }

        sqlx::query!(
            "UPDATE kagzi.step_runs SET is_latest = false WHERE run_id = $1 AND step_id = $2 AND is_latest = true",
            params.run_id,
            params.step_id
        )
        .execute(&mut *tx)
        .await?;

        let retry_policy_json = params.retry_policy.map(serde_json::to_value).transpose()?;
        let attempt_id = Uuid::now_v7();

        sqlx::query!(
            r#"
            INSERT INTO kagzi.step_runs (attempt_id, run_id, step_id, step_kind, status, input, started_at, is_latest, attempt_number, namespace_id, retry_policy)
            VALUES ($1, $2, $3, $4, 'RUNNING', $5, NOW(), true, 
                    COALESCE((SELECT MAX(attempt_number) FROM kagzi.step_runs WHERE run_id = $2 AND step_id = $3), 0) + 1,
                    (SELECT namespace_id FROM kagzi.workflow_runs WHERE run_id = $2),
                    $6)
            "#,
            attempt_id,
            params.run_id,
            params.step_id,
            params.step_kind.as_ref(),
            params.input,
            retry_policy_json
        )
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        Ok(BeginStepResult {
            should_execute: true,
            cached_output: None,
        })
    }

    #[instrument(skip(self, output))]
    async fn complete(
        &self,
        run_id: Uuid,
        step_id: &str,
        output: serde_json::Value,
    ) -> Result<(), StoreError> {
        let mut tx = self.pool.begin().await?;

        let step_kind = self.latest_step_kind_strict(run_id, step_id).await?;

        let result = sqlx::query!(
            r#"
            UPDATE kagzi.step_runs 
            SET status = 'COMPLETED', output = $3, finished_at = NOW()
            WHERE run_id = $1 AND step_id = $2 AND is_latest = true AND status = 'RUNNING'
            "#,
            run_id,
            step_id,
            &output
        )
        .execute(&mut *tx)
        .await?;

        if result.rows_affected() == 0 {
            sqlx::query!(
                "UPDATE kagzi.step_runs SET is_latest = false WHERE run_id = $1 AND step_id = $2 AND is_latest = true",
                run_id,
                step_id
            )
            .execute(&mut *tx)
            .await?;

            self.insert_step_result(
                &mut tx,
                StepResultInsert {
                    run_id,
                    step_id,
                    step_kind,
                    status: "COMPLETED",
                    output: Some(&output),
                    error: None,
                },
            )
            .await?;
        }

        tx.commit().await?;

        Ok(())
    }

    #[instrument(skip(self, params))]
    async fn fail(&self, params: FailStepParams) -> Result<FailStepResult, StoreError> {
        let retry_info = self.get_retry_info(params.run_id, &params.step_id).await?;

        if let Some(info) = retry_info {
            let policy = info.retry_policy.unwrap_or_default();

            let should_retry = !params.non_retryable
                && !policy.is_non_retryable(&params.error)
                && policy.should_retry(info.attempt_number);

            if should_retry {
                let delay_ms = params
                    .retry_after_ms
                    .unwrap_or_else(|| policy.calculate_delay_ms(info.attempt_number));

                sqlx::query!(
                    r#"
                    UPDATE kagzi.step_runs
                    SET status = 'PENDING',
                        retry_at = NOW() + ($3 * INTERVAL '1 millisecond'),
                        error = $4
                    WHERE run_id = $1 AND step_id = $2 AND is_latest = true
                    "#,
                    params.run_id,
                    params.step_id,
                    delay_ms as f64,
                    params.error
                )
                .execute(&self.pool)
                .await?;

                let retry_at = Utc::now() + chrono::Duration::milliseconds(delay_ms);
                return Ok(FailStepResult {
                    scheduled_retry: true,
                    retry_at: Some(retry_at),
                });
            }
        }

        let mut tx = self.pool.begin().await?;

        let step_kind = self
            .latest_step_kind_strict(params.run_id, &params.step_id)
            .await?;

        let result = sqlx::query!(
            r#"
            UPDATE kagzi.step_runs 
            SET status = 'FAILED', error = $3, finished_at = NOW()
            WHERE run_id = $1 AND step_id = $2 AND is_latest = true AND status = 'RUNNING'
            "#,
            params.run_id,
            params.step_id,
            params.error
        )
        .execute(&mut *tx)
        .await?;

        if result.rows_affected() == 0 {
            sqlx::query!(
                "UPDATE kagzi.step_runs SET is_latest = false WHERE run_id = $1 AND step_id = $2 AND is_latest = true",
                params.run_id,
                params.step_id
            )
            .execute(&mut *tx)
            .await?;

            self.insert_step_result(
                &mut tx,
                StepResultInsert {
                    run_id: params.run_id,
                    step_id: &params.step_id,
                    step_kind,
                    status: "FAILED",
                    output: None,
                    error: Some(&params.error),
                },
            )
            .await?;
        }

        tx.commit().await?;

        Ok(FailStepResult {
            scheduled_retry: false,
            retry_at: None,
        })
    }

    #[instrument(skip(self))]
    async fn process_pending_retries(&self) -> Result<Vec<RetryTriggered>, StoreError> {
        let rows: Vec<RetryTriggeredRow> = sqlx::query_as!(
            RetryTriggeredRow,
            r#"
            UPDATE kagzi.step_runs
            SET status = 'PENDING', retry_at = NULL
            WHERE attempt_id IN (
                SELECT attempt_id
                FROM kagzi.step_runs
                WHERE status = 'PENDING'
                  AND retry_at IS NOT NULL
                  AND retry_at <= NOW()
                FOR UPDATE SKIP LOCKED
                LIMIT 100
            )
            RETURNING run_id as "run_id!", step_id as "step_id!", attempt_number
            "#
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| RetryTriggered {
                run_id: r.run_id,
                step_id: r.step_id,
                attempt_number: r.attempt_number,
            })
            .collect())
    }
}
