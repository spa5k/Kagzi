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
use crate::repository::WorkflowScheduleRepository;

const SCHEDULE_COLUMNS: &str = "\
    schedule_id, namespace_id, task_queue, workflow_type, cron_expr, \
    input, context, enabled, max_catchup, next_fire_at, last_fired_at, \
    version, created_at, updated_at";

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

        let schedule_id: Uuid = sqlx::query_scalar!(
            r#"
            INSERT INTO kagzi.schedules (
                namespace_id, task_queue, workflow_type, cron_expr, input, context,
                enabled, max_catchup, next_fire_at, last_fired_at, version
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, NULL, $10)
            RETURNING schedule_id
            "#,
            params.namespace_id,
            params.task_queue,
            params.workflow_type,
            params.cron_expr,
            params.input,
            params.context,
            params.enabled,
            max_catchup,
            params.next_fire_at,
            params.version
        )
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
        let row = sqlx::query_as!(
            Schedule,
            r#"
            SELECT schedule_id, namespace_id, task_queue, workflow_type, cron_expr,
                   input, context, enabled, max_catchup, next_fire_at, last_fired_at,
                   version, created_at, updated_at
            FROM kagzi.schedules
            WHERE schedule_id = $1 AND namespace_id = $2
            "#,
            id,
            namespace_id
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    #[instrument(skip(self, params))]
    async fn list(
        &self,
        params: ListSchedulesParams,
    ) -> Result<PaginatedResult<Schedule, ScheduleCursor>, StoreError> {
        let page_size = params.page_size.max(1) as usize;
        let limit = (page_size + 1) as i64;

        let mut builder = QueryBuilder::new("SELECT ");
        builder.push(SCHEDULE_COLUMNS);
        builder.push(" FROM kagzi.schedules WHERE namespace_id = ");
        builder.push_bind(&params.namespace_id);
        if let Some(task_queue) = &params.task_queue {
            builder.push(" AND task_queue = ").push_bind(task_queue);
        }
        if let Some(ref cursor) = params.cursor {
            builder.push(" AND (created_at, schedule_id) < (");
            builder.push_bind(cursor.created_at);
            builder.push(", ");
            builder.push_bind(cursor.schedule_id);
            builder.push(")");
        }

        builder.push(" ORDER BY created_at DESC, schedule_id DESC LIMIT ");
        builder.push_bind(limit);

        let rows: Vec<Schedule> = builder.build_query_as().fetch_all(&self.pool).await?;

        let has_more = rows.len() > page_size;
        let items: Vec<Schedule> = rows.into_iter().take(page_size).collect();

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

        sqlx::query!(
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
            id,
            namespace_id,
            params.task_queue,
            params.workflow_type,
            params.cron_expr,
            params.input,
            params.context,
            params.enabled,
            max_catchup,
            params.next_fire_at,
            params.version
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    #[instrument(skip(self))]
    async fn delete(&self, id: Uuid, namespace_id: &str) -> Result<bool, StoreError> {
        let result = sqlx::query!(
            r#"
            DELETE FROM kagzi.schedules
            WHERE schedule_id = $1 AND namespace_id = $2
            "#,
            id,
            namespace_id
        )
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }

    #[instrument(skip(self))]
    async fn due_schedules(
        &self,
        now: DateTime<Utc>,
        limit: i64,
    ) -> Result<Vec<Schedule>, StoreError> {
        let rows = sqlx::query_as!(
            Schedule,
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
            now,
            limit
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    #[instrument(skip(self))]
    async fn advance_schedule(
        &self,
        id: Uuid,
        last_fired: DateTime<Utc>,
        next_fire: DateTime<Utc>,
    ) -> Result<(), StoreError> {
        sqlx::query!(
            r#"
            UPDATE kagzi.schedules
            SET last_fired_at = $2,
                next_fire_at = $3,
                updated_at = NOW()
            WHERE schedule_id = $1
            "#,
            id,
            last_fired,
            next_fire
        )
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
        sqlx::query!(
            r#"
            INSERT INTO kagzi.schedule_firings (schedule_id, fire_at, run_id)
            VALUES ($1, $2, $3)
            ON CONFLICT (schedule_id, fire_at) DO NOTHING
            "#,
            schedule_id,
            fire_at,
            run_id
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }
}
