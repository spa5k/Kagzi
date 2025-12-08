use async_trait::async_trait;
use uuid::Uuid;

use crate::error::StoreError;
use crate::models::{
    ListWorkersParams, RegisterWorkerParams, Worker, WorkerCursor, WorkerHeartbeatParams,
    WorkerStatus,
};
use crate::postgres::PaginatedResult;

#[async_trait]
pub trait WorkerRepository: Send + Sync {
    async fn register(&self, params: RegisterWorkerParams) -> Result<Uuid, StoreError>;

    async fn heartbeat(&self, params: WorkerHeartbeatParams) -> Result<bool, StoreError>;

    async fn start_drain(&self, worker_id: Uuid, namespace_id: &str) -> Result<(), StoreError>;

    async fn deregister(&self, worker_id: Uuid, namespace_id: &str) -> Result<(), StoreError>;

    /// Find worker by ID. Worker UUIDs are globally unique.
    async fn find_by_id(&self, worker_id: Uuid) -> Result<Option<Worker>, StoreError>;

    async fn list(
        &self,
        params: ListWorkersParams,
    ) -> Result<PaginatedResult<Worker, WorkerCursor>, StoreError>;

    async fn mark_stale_offline(&self, threshold_secs: i64) -> Result<u64, StoreError>;

    async fn count_online(&self, namespace_id: &str, task_queue: &str) -> Result<i64, StoreError>;

    async fn update_active_count(
        &self,
        worker_id: Uuid,
        namespace_id: &str,
        delta: i32,
    ) -> Result<(), StoreError>;

    async fn count(
        &self,
        namespace_id: &str,
        task_queue: Option<&str>,
        filter_status: Option<WorkerStatus>,
    ) -> Result<i64, StoreError>;
}
