use async_trait::async_trait;
use sqlx::{PgPool, Postgres, QueryBuilder, Transaction};
use tracing::{instrument, warn};
use uuid::Uuid;

use super::StoreConfig;
use crate::error::StoreError;
use crate::models::{
    BeginStepParams, BeginStepResult, FailStepParams, FailStepResult, ListStepsParams,
    PaginatedResult, RetryPolicy, StepCursor, StepKind, StepRetryInfo, StepRun,
};
use crate::repository::StepRepository;

const STEP_COLUMNS: &str = "\
    attempt_id, run_id, step_id, namespace_id, step_kind, attempt_number, status, \
    input, output, error, child_workflow_run_id, created_at, started_at, \
    finished_at, retry_policy";

#[derive(sqlx::FromRow)]
struct StepRunRow {
    attempt_id: Uuid,
    run_id: Uuid,
    step_id: String,
    namespace_id: String,
    step_kind: String,
    attempt_number: i32,
    status: String,
    input: Option<Vec<u8>>,
    output: Option<Vec<u8>>,
    error: Option<String>,
    child_workflow_run_id: Option<Uuid>,
    created_at: Option<chrono::DateTime<chrono::Utc>>,
    started_at: Option<chrono::DateTime<chrono::Utc>>,
    finished_at: Option<chrono::DateTime<chrono::Utc>>,
    retry_policy: Option<serde_json::Value>,
}

struct StepResultInsert<'a> {
    run_id: Uuid,
    step_id: &'a str,
    step_kind: StepKind,
    status: &'a str,
    output: Option<&'a [u8]>,
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
    output: Option<Vec<u8>>,
}

#[derive(Clone)]
pub struct PgStepRepository {
    pool: PgPool,
    config: StoreConfig,
}

impl PgStepRepository {
    pub fn new(pool: PgPool, config: StoreConfig) -> Self {
        Self { pool, config }
    }

    fn validate_payload_size(&self, bytes: &[u8], context: &str) -> Result<(), StoreError> {
        let size = bytes.len();
        if size > self.config.payload_max_size_bytes {
            return Err(StoreError::invalid_argument(format!(
                "{} exceeds maximum size of {} bytes ({} bytes). Do not use Kagzi for blob storage.",
                context, self.config.payload_max_size_bytes, size
            )));
        }

        if size > self.config.payload_warn_threshold_bytes {
            warn!(
                size_bytes = size,
                context = context,
                "Payload exceeds {} bytes. Consider storing large data externally.",
                self.config.payload_warn_threshold_bytes
            );
        }

        Ok(())
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

        if let Some(ref input) = params.input {
            self.validate_payload_size(input, "Step input")?;
        }

        let existing: Option<ExistingStepRow> = sqlx::query_as!(
            ExistingStepRow,
            r#"
            SELECT status, output
            FROM kagzi.step_runs
            WHERE run_id = $1 AND step_id = $2 AND is_latest = true
            "#,
            params.run_id,
            params.step_id
        )
        .fetch_optional(&mut *tx)
        .await?;

        if let Some(ref step) = existing
            && step.status == "COMPLETED"
        {
            return Ok(BeginStepResult {
                should_execute: false,
                cached_output: step.output.clone(),
            });
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
        output: Vec<u8>,
    ) -> Result<(), StoreError> {
        let mut tx = self.pool.begin().await?;

        self.validate_payload_size(&output, "Step output")?;

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
            // Check if step is already completed (idempotent success)
            let existing = sqlx::query!(
                r#"
                SELECT status
                FROM kagzi.step_runs
                WHERE run_id = $1 AND step_id = $2 AND is_latest = true
                "#,
                run_id,
                step_id
            )
            .fetch_optional(&mut *tx)
            .await?;

            if existing.is_some_and(|step| step.status == "COMPLETED") {
                // Already completed, this is idempotent success
                tx.commit().await?;
                return Ok(());
            }

            // Not completed, insert new result
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
                    output: Some(output.as_slice()),
                    error: None,
                },
            )
            .await?;
        }

        tx.commit().await?;

        Ok(())
    }

    /// Fail a step, potentially scheduling a workflow retry.
    ///
    /// In the simplified model, step failures trigger workflow retries rather than
    /// step-level retries. When the workflow replays, cached steps return instantly
    /// until the failed step is re-executed.
    #[instrument(skip(self, params))]
    async fn fail(&self, params: FailStepParams) -> Result<FailStepResult, StoreError> {
        let retry_info = self.get_retry_info(params.run_id, &params.step_id).await?;

        // Check if we should schedule a workflow retry
        if let Some(info) = retry_info {
            let policy = info.retry_policy.unwrap_or_default();

            let should_retry = !params.non_retryable
                && !policy.is_non_retryable(&params.error)
                && policy.should_retry(info.attempt_number);

            if should_retry {
                let delay_ms = params
                    .retry_after_ms
                    .unwrap_or_else(|| policy.calculate_delay_ms(info.attempt_number));

                // Mark step as failed
                sqlx::query!(
                    r#"
                    UPDATE kagzi.step_runs
                    SET status = 'FAILED',
                        error = $3,
                        finished_at = NOW()
                    WHERE run_id = $1 AND step_id = $2 AND is_latest = true
                    "#,
                    params.run_id,
                    params.step_id,
                    params.error
                )
                .execute(&self.pool)
                .await?;

                // Signal that workflow should be rescheduled
                return Ok(FailStepResult {
                    scheduled_retry: true,
                    schedule_workflow_retry_ms: Some(delay_ms as u64),
                });
            }
        }

        // No retry - mark step as permanently failed
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
            schedule_workflow_retry_ms: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use sqlx::postgres::PgPoolOptions;

    use super::*;

    fn make_repo(max: usize, warn: usize) -> PgStepRepository {
        let pool = PgPoolOptions::new()
            .connect_lazy("postgres://postgres:postgres@localhost:5432/postgres")
            .expect("connect_lazy should create pool");
        PgStepRepository::new(
            pool,
            StoreConfig {
                payload_warn_threshold_bytes: warn,
                payload_max_size_bytes: max,
            },
        )
    }

    #[tokio::test]
    async fn step_payload_rejects_when_over_limit() {
        let repo = make_repo(2, 1);
        let data = vec![0u8; 3];
        let err = repo
            .validate_payload_size(&data, "Step input")
            .expect_err("should reject oversized payload");
        assert!(matches!(err, StoreError::InvalidArgument { .. }));
    }
}
