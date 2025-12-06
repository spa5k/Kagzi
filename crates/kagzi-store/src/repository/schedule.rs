use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::error::StoreError;
use crate::models::{CreateSchedule, ListSchedulesParams, Schedule, UpdateSchedule};

#[async_trait]
pub trait ScheduleRepository: Send + Sync {
    async fn create(&self, params: CreateSchedule) -> Result<Uuid, StoreError>;

    async fn find_by_id(
        &self,
        id: Uuid,
        namespace_id: &str,
    ) -> Result<Option<Schedule>, StoreError>;

    async fn list(&self, params: ListSchedulesParams) -> Result<Vec<Schedule>, StoreError>;

    async fn update(
        &self,
        id: Uuid,
        namespace_id: &str,
        params: UpdateSchedule,
    ) -> Result<(), StoreError>;

    async fn delete(&self, id: Uuid, namespace_id: &str) -> Result<bool, StoreError>;

    async fn due_schedules(
        &self,
        now: DateTime<Utc>,
        limit: i32,
    ) -> Result<Vec<Schedule>, StoreError>;

    async fn advance_schedule(
        &self,
        id: Uuid,
        last_fired: DateTime<Utc>,
        next_fire: DateTime<Utc>,
    ) -> Result<(), StoreError>;

    async fn record_firing(
        &self,
        schedule_id: Uuid,
        fire_at: DateTime<Utc>,
        run_id: Uuid,
    ) -> Result<(), StoreError>;
}
