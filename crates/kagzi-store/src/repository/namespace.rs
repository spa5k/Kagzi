use async_trait::async_trait;

use crate::error::StoreError;
use crate::models::{CreateNamespace, Namespace, PaginatedResult, ResourceCount};

/// Repository trait for namespace persistence operations.
///
/// Provides CRUD operations for namespaces with support for soft-deletion
/// and resource counting for safe deletion validation.
#[async_trait]
pub trait NamespaceRepository: Send + Sync {
    /// Create a new namespace.
    async fn create(&self, params: CreateNamespace) -> Result<Namespace, StoreError>;

    /// Find a namespace by its ID.
    /// Returns None if not found or soft-deleted (unless include_deleted is true).
    async fn find_by_id(
        &self,
        namespace_id: &str,
        include_deleted: bool,
    ) -> Result<Option<Namespace>, StoreError>;

    /// List all namespaces with pagination.
    async fn list(
        &self,
        page_size: i32,
        cursor: Option<String>,
        include_deleted: bool,
    ) -> Result<PaginatedResult<Namespace, String>, StoreError>;

    /// Update a namespace's display name and/or description.
    async fn update(
        &self,
        namespace_id: &str,
        display_name: Option<String>,
        description: Option<String>,
    ) -> Result<Namespace, StoreError>;

    /// Hard delete a namespace (permanently removes from database).
    /// Should only be called after CASCADE deletion of all resources.
    async fn delete(&self, namespace_id: &str) -> Result<bool, StoreError>;

    /// Soft delete a namespace (sets deleted_at timestamp).
    async fn soft_delete(&self, namespace_id: &str) -> Result<bool, StoreError>;

    /// Count dependent resources (workflows, workers, schedules) in a namespace.
    /// Used for FAIL_IF_RESOURCES deletion mode validation.
    async fn count_resources(&self, namespace_id: &str) -> Result<ResourceCount, StoreError>;

    /// List all distinct namespace IDs that exist in the system.
    /// Used for backwards compatibility with existing implicit namespaces.
    async fn list_distinct_namespaces(&self) -> Result<Vec<String>, StoreError>;

    /// Get a namespace by ID, or create it if it doesn't exist.
    /// Returns the namespace (either existing or newly created).
    async fn get_or_create(&self, namespace_id: &str) -> Result<Namespace, StoreError>;
}
