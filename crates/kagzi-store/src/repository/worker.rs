use async_trait::async_trait;
use uuid::Uuid;

use crate::error::StoreError;
use crate::models::{
    ListWorkersParams, PaginatedResult, RegisterWorkerParams, Worker, WorkerCursor,
    WorkerHeartbeatParams, WorkerStatus,
};

#[async_trait]
pub trait WorkerRepository: Send + Sync {
    async fn register(&self, params: RegisterWorkerParams) -> Result<Uuid, StoreError>;

    async fn heartbeat(&self, params: WorkerHeartbeatParams) -> Result<bool, StoreError>;

    async fn start_drain(&self, worker_id: Uuid, namespace: &str) -> Result<(), StoreError>;

    async fn deregister(&self, worker_id: Uuid, namespace: &str) -> Result<(), StoreError>;

    async fn find_by_id(&self, worker_id: Uuid) -> Result<Option<Worker>, StoreError>;

    async fn list(
        &self,
        params: ListWorkersParams,
    ) -> Result<PaginatedResult<Worker, WorkerCursor>, StoreError>;

    async fn mark_stale_offline(&self, threshold_secs: i64) -> Result<u64, StoreError>;

    async fn count_online(&self, namespace: &str, task_queue: &str) -> Result<i64, StoreError>;

    async fn count(
        &self,
        namespace: &str,
        task_queue: Option<&str>,
        filter_status: Option<WorkerStatus>,
    ) -> Result<i64, StoreError>;

    /// Get list of distinct namespaces from workers table.
    /// Useful for status reporting and administration.
    async fn list_distinct_namespaces(&self) -> Result<Vec<String>, StoreError>;
}
