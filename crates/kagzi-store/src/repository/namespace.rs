use async_trait::async_trait;

use crate::error::StoreError;
use crate::models::{
    CreateNamespace, Namespace, NamespaceStats, PaginatedResult, UpdateNamespace, WorkflowTypeInfo,
};

/// Repository trait for namespace persistence operations.
///
/// Namespaces are enabled/disabled, not deleted. Provides CRUD operations
/// and statistics for namespace management.
#[async_trait]
pub trait NamespaceRepository: Send + Sync {
    /// Create a new namespace (enabled by default).
    async fn create(&self, params: CreateNamespace) -> Result<Namespace, StoreError>;

    /// Find a namespace by its identifier.
    async fn find_by_id(&self, namespace: &str) -> Result<Option<Namespace>, StoreError>;

    /// Find a namespace by internal UUID.
    async fn find_by_internal_id(&self, id: uuid::Uuid) -> Result<Option<Namespace>, StoreError>;

    /// List all namespaces with pagination.
    async fn list(
        &self,
        page_size: i32,
        cursor: Option<String>,
    ) -> Result<PaginatedResult<Namespace, String>, StoreError>;

    /// Update a namespace's display name and/or description.
    async fn update(
        &self,
        namespace: &str,
        params: UpdateNamespace,
    ) -> Result<Namespace, StoreError>;

    /// Enable a namespace (allows workflows to run).
    async fn enable(&self, namespace: &str) -> Result<bool, StoreError>;

    /// Disable a namespace (prevents new workflows from starting).
    async fn disable(&self, namespace: &str) -> Result<bool, StoreError>;

    /// Get statistics for a namespace.
    async fn get_stats(&self, namespace: &str) -> Result<NamespaceStats, StoreError>;

    /// List workflow types with their statistics in a namespace.
    async fn list_workflow_types(
        &self,
        namespace: &str,
        page_size: i32,
        cursor: Option<String>,
    ) -> Result<PaginatedResult<WorkflowTypeInfo, String>, StoreError>;

    /// List all distinct namespace identifiers.
    async fn list_distinct_namespaces(&self) -> Result<Vec<String>, StoreError>;

    /// Get a namespace by identifier, or create it if it doesn't exist.
    /// Returns the namespace (either existing or newly created).
    async fn get_or_create(&self, namespace: &str) -> Result<Namespace, StoreError>;
}
