use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::StoreError;
use crate::models::{CreateSchedule, ListSchedulesParams, Schedule, UpdateSchedule};
use crate::repository::ScheduleRepository;

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
    /// Converts a `ScheduleRow` into the public `Schedule` model.
    ///
    /// This consumes the row and returns a `Schedule` with the same field values.
    ///
    /// # Examples
    ///
    /// ```
    /// // Given a `ScheduleRow` named `row`, convert it to the public model:
    /// let schedule = row.into_model();
    /// ```
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
    /// Creates a new `PgScheduleRepository` that uses the provided Postgres connection pool.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use kagzi_store::postgres::schedule::PgScheduleRepository;
    /// use sqlx::PgPool;
    ///
    /// // Assume `pool` is a valid `PgPool` obtained elsewhere.
    /// let pool: PgPool = /* obtain pool */ todo!();
    /// let repo = PgScheduleRepository::new(pool);
    /// ```
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Clamp a max-catchup value to the allowed range.
    ///
    /// Clamps the given `max_catchup` to the inclusive range 1 to 10_000.
    ///
    /// # Examples
    ///
    /// ```
    /// assert_eq!(crate::clamp_max_catchup(0), 1);
    /// assert_eq!(crate::clamp_max_catchup(42), 42);
    /// assert_eq!(crate::clamp_max_catchup(20_000), 10_000);
    /// ```
    fn clamp_max_catchup(max_catchup: i32) -> i32 {
        max_catchup.clamp(1, 10_000)
    }
}

#[async_trait]
impl ScheduleRepository for PgScheduleRepository {
    /// Inserts a new schedule into the database and returns its generated schedule ID.
    ///
    /// The provided `CreateSchedule` is normalized (its `max_catchup` is clamped) before insertion.
    ///
    /// # Returns
    ///
    /// `Uuid` of the newly created schedule.
    ///
    /// # Examples
    ///
    /// ```
    /// use uuid::Uuid;
    /// use chrono::Utc;
    /// use serde_json::json;
    /// // Construct a repository `repo` and a `pool` previously; omitted for brevity.
    /// let repo = /* PgScheduleRepository::new(pool) */ unimplemented!();
    /// let params = CreateSchedule {
    ///     namespace_id: "default".into(),
    ///     task_queue: "main".into(),
    ///     workflow_type: "example".into(),
    ///     cron_expr: "0 * * * *".into(),
    ///     input: json!({}),
    ///     context: None,
    ///     enabled: true,
    ///     max_catchup: 10,
    ///     next_fire_at: Utc::now(),
    ///     version: None,
    /// };
    /// let id: Uuid = futures::executor::block_on(async { repo.create(params).await.unwrap() });
    /// assert!(!id.to_string().is_empty());
    /// ```
    async fn create(&self, params: CreateSchedule) -> Result<Uuid, StoreError> {
        let max_catchup = Self::clamp_max_catchup(params.max_catchup);

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

    /// Fetches a schedule by its ID within the specified namespace.
    ///
    /// Returns `Some(Schedule)` if a schedule with the given `id` exists in `namespace_id`, `None` otherwise.
    /// The `Result` is `Err` if the database query fails.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use uuid::Uuid;
    ///
    /// let repo = PgScheduleRepository::new(pool);
    /// let id = Uuid::parse_str("3fa85f64-5717-4562-b3fc-2c963f66afa6").unwrap();
    /// let schedule = repo.find_by_id(id, "default").await.unwrap();
    /// if let Some(s) = schedule {
    ///     println!("found schedule {}", s.schedule_id);
    /// }
    /// ```
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

    /// Lists schedules in a namespace, optionally filtered by task queue.
    ///
    /// When `params.task_queue` is provided, only schedules for that task queue are returned;
    /// otherwise all schedules in the namespace are returned. Results are ordered by
    /// `created_at` descending then `schedule_id` descending. If `params.limit` is `None`,
    /// a default limit of 100 is applied.
    ///
    /// # Parameters
    ///
    /// - `params`: query parameters containing `namespace_id`, optional `task_queue`, and optional `limit`.
    ///
    /// # Returns
    ///
    /// A vector of schedules matching the query, ordered and limited as described.
    ///
    /// # Examples
    ///
    /// ```
    /// # use uuid::Uuid;
    /// # use chrono::Utc;
    /// # use crate::store::{ListSchedulesParams, Schedule};
    /// # async fn example(repo: &impl crate::store::ScheduleRepository) -> Result<(), crate::store::StoreError> {
    /// let params = ListSchedulesParams {
    ///     namespace_id: "default".to_string(),
    ///     task_queue: None,
    ///     limit: Some(50),
    /// };
    /// let schedules: Vec<Schedule> = repo.list(params).await?;
    /// assert!(schedules.len() <= 50);
    /// # Ok(())
    /// # }
    /// ```
    async fn list(&self, params: ListSchedulesParams) -> Result<Vec<Schedule>, StoreError> {
        let rows = match params.task_queue {
            Some(ref task_queue) => {
                sqlx::query_as::<_, ScheduleRow>(
                    r#"
                    SELECT schedule_id, namespace_id, task_queue, workflow_type, cron_expr,
                           input, context, enabled, max_catchup, next_fire_at, last_fired_at,
                           version, created_at, updated_at
                    FROM kagzi.schedules
                    WHERE namespace_id = $1 AND task_queue = $2
                    ORDER BY created_at DESC, schedule_id DESC
                    LIMIT $3
                    "#,
                )
                .bind(&params.namespace_id)
                .bind(task_queue)
                .bind(params.limit.unwrap_or(100))
                .fetch_all(&self.pool)
                .await?
            }
            None => {
                sqlx::query_as::<_, ScheduleRow>(
                    r#"
                    SELECT schedule_id, namespace_id, task_queue, workflow_type, cron_expr,
                           input, context, enabled, max_catchup, next_fire_at, last_fired_at,
                           version, created_at, updated_at
                    FROM kagzi.schedules
                    WHERE namespace_id = $1
                    ORDER BY created_at DESC, schedule_id DESC
                    LIMIT $2
                    "#,
                )
                .bind(&params.namespace_id)
                .bind(params.limit.unwrap_or(100))
                .fetch_all(&self.pool)
                .await?
            }
        };

        Ok(rows.into_iter().map(|r| r.into_model()).collect())
    }

    /// Updates an existing schedule's fields in storage using the provided `params`; any `None` fields in `params` leave the corresponding column unchanged.
    ///
    /// The `max_catchup` value within `params` will be clamped to the repository's allowed range if present.
    ///
    /// # Parameters
    ///
    /// - `id`: Identifier of the schedule to update.
    /// - `namespace_id`: Namespace owning the schedule.
    /// - `params`: Partial update fields; `None` values do not modify the existing columns.
    ///
    /// # Returns
    ///
    /// `Ok(())` on success, `Err(StoreError)` if the update fails.
    ///
    /// # Examples
    ///
    /// ```
    /// # use uuid::Uuid;
    /// # use chrono::Utc;
    /// # async fn _example(repo: &crate::postgres::schedule::PgScheduleRepository) -> Result<(), crate::store::StoreError> {
    /// let id = Uuid::new_v4();
    /// let namespace = "default";
    /// let params = crate::models::UpdateSchedule {
    ///     task_queue: Some("queue-a".to_string()),
    ///     workflow_type: None,
    ///     cron_expr: None,
    ///     input: None,
    ///     context: None,
    ///     enabled: None,
    ///     max_catchup: Some(10),
    ///     next_fire_at: None,
    ///     version: Some("v2".to_string()),
    /// };
    ///
    /// repo.update(id, namespace, params).await?;
    /// # Ok(())
    /// # }
    /// ```
    async fn update(
        &self,
        id: Uuid,
        namespace_id: &str,
        params: UpdateSchedule,
    ) -> Result<(), StoreError> {
        let max_catchup = params.max_catchup.map(Self::clamp_max_catchup);

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

    /// Deletes a schedule by its ID within the given namespace.
    ///
    /// # Examples
    ///
    /// ```
    /// # use uuid::Uuid;
    /// # async fn example(repo: &crate::postgres::PgScheduleRepository) {
    /// let id = Uuid::new_v4();
    /// let deleted = repo.delete(id, "default").await.unwrap();
    /// // `deleted` is `true` if a matching schedule existed and was removed, `false` otherwise.
    /// # }
    /// ```
    ///
    /// # Returns
    ///
    /// `true` if a schedule with the given `id` and `namespace_id` was deleted, `false` otherwise.
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

    /// Selects enabled schedules whose `next_fire_at` is at or before the given time and locks them for processing.
    ///
    /// The selected rows are ordered by `next_fire_at` ascending and returned up to the provided `limit`. Rows are locked with `FOR UPDATE SKIP LOCKED` to allow safe concurrent processing.
    ///
    /// # Examples
    ///
    /// ```
    /// # use chrono::Utc;
    /// # async fn example(repo: &crate::PgScheduleRepository) {
    /// let now = Utc::now();
    /// let due = repo.due_schedules(now, 10).await.unwrap();
    /// assert!(due.iter().all(|s| s.enabled));
    /// # }
    /// ```
    ///
    /// # Returns
    ///
    /// `Vec<Schedule>` containing the selected schedules in ascending order of `next_fire_at`.
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

    /// Advance a schedule's firing timestamps and update its modification time.
    ///
    /// Updates the schedule identified by `id` to set `last_fired_at` to `last_fired`,
    /// `next_fire_at` to `next_fire`, and `updated_at` to the current time in the database.
    ///
    /// # Returns
    ///
    /// `()` on success.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use uuid::Uuid;
    /// # use chrono::{Utc, Duration};
    /// # use crate::postgres::schedule::PgScheduleRepository;
    /// #[tokio::test]
    /// async fn example_advance_schedule() {
    ///     // Obtain a PgPool and construct the repository (omitted).
    ///     let pool = /* PgPool */ unimplemented!();
    ///     let repo = PgScheduleRepository::new(pool);
    ///
    ///     let schedule_id = Uuid::new_v4();
    ///     let last_fired = Utc::now();
    ///     let next_fire = last_fired + Duration::hours(1);
    ///
    ///     repo.advance_schedule(schedule_id, last_fired, next_fire).await.unwrap();
    /// }
    /// ```
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

    /// Record that a schedule fired at a specific time by inserting a firing record.
    ///
    /// Inserts a row into `kagzi.schedule_firings` for the given `schedule_id` and `fire_at`.
    /// If a record for the same `(schedule_id, fire_at)` already exists, the insert is ignored.
    ///
    /// # Returns
    ///
    /// `Ok(())` on success, `Err(StoreError)` if the database operation fails.
    ///
    /// # Examples
    ///
    /// ```
    /// # use uuid::Uuid;
    /// # use chrono::Utc;
    /// # async fn example(repo: &crate::postgres::schedule::PgScheduleRepository) -> Result<(), crate::store::StoreError> {
    /// let schedule_id = Uuid::new_v4();
    /// let run_id = Uuid::new_v4();
    /// let fire_at = Utc::now();
    /// repo.record_firing(schedule_id, fire_at, run_id).await?;
    /// # Ok(())
    /// # }
    /// ```
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