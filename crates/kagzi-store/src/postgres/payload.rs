use async_trait::async_trait;
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

use crate::error::StoreError;
use crate::models::WorkflowPayload;
use crate::repository::PayloadRepository;

#[derive(FromRow)]
struct PayloadRow {
    run_id: Uuid,
    input: serde_json::Value,
    output: Option<serde_json::Value>,
    context: Option<serde_json::Value>,
}

#[derive(Clone)]
pub struct PgPayloadRepository {
    pool: PgPool,
}

impl PgPayloadRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl PayloadRepository for PgPayloadRepository {
    async fn get(&self, run_id: Uuid) -> Result<Option<WorkflowPayload>, StoreError> {
        let row = sqlx::query_as::<_, PayloadRow>(
            r#"
            SELECT run_id, input, output, context
            FROM kagzi.workflow_payloads
            WHERE run_id = $1
            "#,
        )
        .bind(run_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| WorkflowPayload {
            run_id: r.run_id,
            input: r.input,
            output: r.output,
            context: r.context,
        }))
    }

    async fn upsert(&self, payload: WorkflowPayload) -> Result<(), StoreError> {
        sqlx::query(
            r#"
            INSERT INTO kagzi.workflow_payloads (run_id, input, output, context)
            VALUES ($1, $2, $3, $4)
            ON CONFLICT (run_id) DO UPDATE SET
                input = EXCLUDED.input,
                output = EXCLUDED.output,
                context = EXCLUDED.context
            "#,
        )
        .bind(payload.run_id)
        .bind(payload.input)
        .bind(payload.output)
        .bind(payload.context)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn set_output(&self, run_id: Uuid, output: serde_json::Value) -> Result<(), StoreError> {
        sqlx::query(
            r#"
            UPDATE kagzi.workflow_payloads
            SET output = $2
            WHERE run_id = $1
            "#,
        )
        .bind(run_id)
        .bind(output)
        .execute(&self.pool)
        .await?;

        Ok(())
    }
}
