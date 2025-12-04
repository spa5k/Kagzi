use async_trait::async_trait;
use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::StoreError;
use crate::models::{
    BeginStepParams, BeginStepResult, FailStepParams, FailStepResult, RetryPolicy, RetryTriggered,
    StepRetryInfo, StepRun, StepStatus,
};
use crate::repository::StepRepository;

#[derive(sqlx::FromRow)]
struct StepRunRow {
    attempt_id: Uuid,
    run_id: Uuid,
    step_id: String,
    namespace_id: String,
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

impl StepRunRow {
    fn into_model(self) -> StepRun {
        StepRun {
            attempt_id: self.attempt_id,
            run_id: self.run_id,
            step_id: self.step_id,
            namespace_id: self.namespace_id,
            attempt_number: self.attempt_number,
            status: StepStatus::from_db_str(&self.status),
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
        }
    }
}

#[derive(Clone)]
pub struct PgStepRepository {
    pool: PgPool,
}

impl PgStepRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl StepRepository for PgStepRepository {
    async fn find_by_id(&self, attempt_id: Uuid) -> Result<Option<StepRun>, StoreError> {
        let row = sqlx::query_as::<_, StepRunRow>(
            r#"
            SELECT attempt_id, run_id, step_id, namespace_id, attempt_number, status,
                   input, output, error, child_workflow_run_id,
                   created_at, started_at, finished_at, retry_at, retry_policy
            FROM kagzi.step_runs
            WHERE attempt_id = $1
            "#,
        )
        .bind(attempt_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| r.into_model()))
    }

    async fn find_latest(
        &self,
        run_id: Uuid,
        step_id: &str,
    ) -> Result<Option<StepRun>, StoreError> {
        let row = sqlx::query_as::<_, StepRunRow>(
            r#"
            SELECT attempt_id, run_id, step_id, namespace_id, attempt_number, status,
                   input, output, error, child_workflow_run_id,
                   created_at, started_at, finished_at, retry_at, retry_policy
            FROM kagzi.step_runs
            WHERE run_id = $1 AND step_id = $2 AND is_latest = true
            "#,
        )
        .bind(run_id)
        .bind(step_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| r.into_model()))
    }

    async fn list_by_workflow(
        &self,
        run_id: Uuid,
        step_id: Option<&str>,
        limit: i32,
    ) -> Result<Vec<StepRun>, StoreError> {
        let rows: Vec<StepRunRow> = match step_id {
            None => {
                sqlx::query_as(
                    r#"
                    SELECT attempt_id, run_id, step_id, namespace_id, attempt_number, status,
                           input, output, error, child_workflow_run_id,
                           created_at, started_at, finished_at, retry_at, retry_policy
                    FROM kagzi.step_runs
                    WHERE run_id = $1
                    ORDER BY created_at ASC, attempt_number ASC
                    LIMIT $2
                    "#,
                )
                .bind(run_id)
                .bind(limit as i64)
                .fetch_all(&self.pool)
                .await?
            }
            Some(sid) => {
                sqlx::query_as(
                    r#"
                    SELECT attempt_id, run_id, step_id, namespace_id, attempt_number, status,
                           input, output, error, child_workflow_run_id,
                           created_at, started_at, finished_at, retry_at, retry_policy
                    FROM kagzi.step_runs
                    WHERE run_id = $1 AND step_id = $2
                    ORDER BY attempt_number ASC
                    LIMIT $3
                    "#,
                )
                .bind(run_id)
                .bind(sid)
                .bind(limit as i64)
                .fetch_all(&self.pool)
                .await?
            }
        };

        Ok(rows.into_iter().map(|r| r.into_model()).collect())
    }

    async fn get_retry_info(
        &self,
        run_id: Uuid,
        step_id: &str,
    ) -> Result<Option<StepRetryInfo>, StoreError> {
        let row = sqlx::query!(
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

    async fn begin(&self, params: BeginStepParams) -> Result<BeginStepResult, StoreError> {
        let existing = sqlx::query!(
            r#"
            SELECT status, output, retry_at
            FROM kagzi.step_runs
            WHERE run_id = $1 AND step_id = $2 AND is_latest = true
            "#,
            params.run_id,
            params.step_id
        )
        .fetch_optional(&self.pool)
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
        .execute(&self.pool)
        .await?;

        let retry_policy_json = params.retry_policy.map(serde_json::to_value).transpose()?;

        sqlx::query!(
            r#"
            INSERT INTO kagzi.step_runs (run_id, step_id, status, input, started_at, is_latest, attempt_number, namespace_id, retry_policy)
            VALUES ($1, $2, 'RUNNING', $3, NOW(), true, 
                    COALESCE((SELECT MAX(attempt_number) FROM kagzi.step_runs WHERE run_id = $1 AND step_id = $2), 0) + 1,
                    COALESCE((SELECT namespace_id FROM kagzi.workflow_runs WHERE run_id = $1), 'default'),
                    $4)
            "#,
            params.run_id,
            params.step_id,
            params.input,
            retry_policy_json
        )
        .execute(&self.pool)
        .await?;

        Ok(BeginStepResult {
            should_execute: true,
            cached_output: None,
        })
    }

    async fn complete(
        &self,
        run_id: Uuid,
        step_id: &str,
        output: serde_json::Value,
    ) -> Result<(), StoreError> {
        let result = sqlx::query!(
            r#"
            UPDATE kagzi.step_runs 
            SET status = 'COMPLETED', output = $3, finished_at = NOW()
            WHERE run_id = $1 AND step_id = $2 AND is_latest = true AND status = 'RUNNING'
            "#,
            run_id,
            step_id,
            output
        )
        .execute(&self.pool)
        .await?;

        if result.rows_affected() == 0 {
            sqlx::query!(
                "UPDATE kagzi.step_runs SET is_latest = false WHERE run_id = $1 AND step_id = $2 AND is_latest = true",
                run_id,
                step_id
            )
            .execute(&self.pool)
            .await?;

            sqlx::query!(
                r#"
                INSERT INTO kagzi.step_runs (run_id, step_id, status, output, finished_at, is_latest, attempt_number, namespace_id)
                VALUES ($1, $2, 'COMPLETED', $3, NOW(), true, 
                        COALESCE((SELECT MAX(attempt_number) FROM kagzi.step_runs WHERE run_id = $1 AND step_id = $2), 0) + 1,
                        COALESCE((SELECT namespace_id FROM kagzi.workflow_runs WHERE run_id = $1), 'default'))
                "#,
                run_id,
                step_id,
                output
            )
            .execute(&self.pool)
            .await?;
        }

        Ok(())
    }

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
        .execute(&self.pool)
        .await?;

        if result.rows_affected() == 0 {
            sqlx::query!(
                "UPDATE kagzi.step_runs SET is_latest = false WHERE run_id = $1 AND step_id = $2 AND is_latest = true",
                params.run_id,
                params.step_id
            )
            .execute(&self.pool)
            .await?;

            sqlx::query!(
                r#"
                INSERT INTO kagzi.step_runs (run_id, step_id, status, error, finished_at, is_latest, attempt_number, namespace_id)
                VALUES ($1, $2, 'FAILED', $3, NOW(), true, 
                        COALESCE((SELECT MAX(attempt_number) FROM kagzi.step_runs WHERE run_id = $1 AND step_id = $2), 0) + 1,
                        COALESCE((SELECT namespace_id FROM kagzi.workflow_runs WHERE run_id = $1), 'default'))
                "#,
                params.run_id,
                params.step_id,
                params.error
            )
            .execute(&self.pool)
            .await?;
        }

        Ok(FailStepResult {
            scheduled_retry: false,
            retry_at: None,
        })
    }

    async fn process_pending_retries(&self) -> Result<Vec<RetryTriggered>, StoreError> {
        let rows = sqlx::query!(
            r#"
            UPDATE kagzi.step_runs
            SET status = 'PENDING', retry_at = NULL
            WHERE status = 'PENDING'
              AND retry_at IS NOT NULL
              AND retry_at <= NOW()
            RETURNING run_id, step_id, attempt_number
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
