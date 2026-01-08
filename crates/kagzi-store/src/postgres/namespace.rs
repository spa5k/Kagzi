use async_trait::async_trait;
use chrono::Utc;
use sqlx::PgPool;

use crate::error::StoreError;
use crate::models::{CreateNamespace, Namespace, PaginatedResult, ResourceCount};
use crate::repository::NamespaceRepository;

#[derive(Clone)]
pub struct PgNamespaceRepository {
    pool: PgPool,
}

impl PgNamespaceRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl NamespaceRepository for PgNamespaceRepository {
    async fn create(&self, params: CreateNamespace) -> Result<Namespace, StoreError> {
        let now = Utc::now();
        let namespace = sqlx::query_as!(
            Namespace,
            r#"
            INSERT INTO kagzi.namespaces (namespace_id, display_name, description, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $4)
            RETURNING namespace_id, display_name, description, created_at, updated_at, deleted_at
            "#,
            params.namespace_id,
            params.display_name,
            params.description,
            now
        )
        .fetch_one(&self.pool)
        .await
        .map_err(|e| {
            if let sqlx::Error::Database(db_err) = &e
                && db_err.is_unique_violation()
            {
                return StoreError::Conflict {
                    message: format!("Namespace '{}' already exists", params.namespace_id),
                };
            }
            StoreError::from(e)
        })?;

        Ok(namespace)
    }

    async fn find_by_id(
        &self,
        namespace_id: &str,
        include_deleted: bool,
    ) -> Result<Option<Namespace>, StoreError> {
        if include_deleted {
            sqlx::query_as!(
                Namespace,
                r#"
                SELECT namespace_id, display_name, description, created_at, updated_at, deleted_at
                FROM kagzi.namespaces
                WHERE namespace_id = $1
                "#,
                namespace_id
            )
            .fetch_optional(&self.pool)
            .await
            .map_err(StoreError::from)
        } else {
            sqlx::query_as!(
                Namespace,
                r#"
                SELECT namespace_id, display_name, description, created_at, updated_at, deleted_at
                FROM kagzi.namespaces
                WHERE namespace_id = $1 AND deleted_at IS NULL
                "#,
                namespace_id
            )
            .fetch_optional(&self.pool)
            .await
            .map_err(StoreError::from)
        }
    }

    async fn list(
        &self,
        page_size: i32,
        cursor: Option<String>,
        include_deleted: bool,
    ) -> Result<PaginatedResult<Namespace, String>, StoreError> {
        let limit = page_size.clamp(1, 100);

        let namespaces = if include_deleted {
            if let Some(cursor_id) = cursor {
                sqlx::query_as!(
                    Namespace,
                    r#"
                    SELECT namespace_id, display_name, description, created_at, updated_at, deleted_at
                    FROM kagzi.namespaces
                    WHERE namespace_id > $1
                    ORDER BY namespace_id ASC
                    LIMIT $2
                    "#,
                    cursor_id,
                    limit as i64
                )
                .fetch_all(&self.pool)
                .await
            } else {
                sqlx::query_as!(
                    Namespace,
                    r#"
                    SELECT namespace_id, display_name, description, created_at, updated_at, deleted_at
                    FROM kagzi.namespaces
                    ORDER BY namespace_id ASC
                    LIMIT $1
                    "#,
                    limit as i64
                )
                .fetch_all(&self.pool)
                .await
            }
        } else if let Some(cursor_id) = cursor {
            sqlx::query_as!(
                Namespace,
                r#"
                SELECT namespace_id, display_name, description, created_at, updated_at, deleted_at
                FROM kagzi.namespaces
                WHERE namespace_id > $1 AND deleted_at IS NULL
                ORDER BY namespace_id ASC
                LIMIT $2
                "#,
                cursor_id,
                limit as i64
            )
            .fetch_all(&self.pool)
            .await
        } else {
            sqlx::query_as!(
                Namespace,
                r#"
                SELECT namespace_id, display_name, description, created_at, updated_at, deleted_at
                FROM kagzi.namespaces
                WHERE deleted_at IS NULL
                ORDER BY namespace_id ASC
                LIMIT $1
                "#,
                limit as i64
            )
            .fetch_all(&self.pool)
            .await
        }
        .map_err(StoreError::from)?;

        let has_more = namespaces.len() == limit as usize;
        let next_cursor = namespaces.last().map(|n| n.namespace_id.clone());

        Ok(PaginatedResult {
            items: namespaces,
            next_cursor,
            has_more,
        })
    }

    async fn update(
        &self,
        namespace_id: &str,
        display_name: Option<String>,
        description: Option<String>,
    ) -> Result<Namespace, StoreError> {
        let now = Utc::now();

        // Build dynamic update query based on provided fields
        let namespace = if let (Some(name), Some(desc)) = (&display_name, &description) {
            sqlx::query_as!(
                Namespace,
                r#"
                UPDATE kagzi.namespaces
                SET display_name = $2, description = $3, updated_at = $4
                WHERE namespace_id = $1 AND deleted_at IS NULL
                RETURNING namespace_id, display_name, description, created_at, updated_at, deleted_at
                "#,
                namespace_id,
                name,
                desc,
                now
            )
            .fetch_optional(&self.pool)
            .await
        } else if let Some(name) = &display_name {
            sqlx::query_as!(
                Namespace,
                r#"
                UPDATE kagzi.namespaces
                SET display_name = $2, updated_at = $3
                WHERE namespace_id = $1 AND deleted_at IS NULL
                RETURNING namespace_id, display_name, description, created_at, updated_at, deleted_at
                "#,
                namespace_id,
                name,
                now
            )
            .fetch_optional(&self.pool)
            .await
        } else if let Some(desc) = &description {
            sqlx::query_as!(
                Namespace,
                r#"
                UPDATE kagzi.namespaces
                SET description = $2, updated_at = $3
                WHERE namespace_id = $1 AND deleted_at IS NULL
                RETURNING namespace_id, display_name, description, created_at, updated_at, deleted_at
                "#,
                namespace_id,
                desc,
                now
            )
            .fetch_optional(&self.pool)
            .await
        } else {
            // No fields to update
            return self
                .find_by_id(namespace_id, false)
                .await?
                .ok_or_else(|| StoreError::not_found("namespace", namespace_id));
        }
        .map_err(StoreError::from)?;

        namespace.ok_or_else(|| StoreError::not_found("namespace", namespace_id))
    }

    async fn delete(&self, namespace_id: &str) -> Result<bool, StoreError> {
        let result = sqlx::query!(
            r#"
            DELETE FROM kagzi.namespaces
            WHERE namespace_id = $1
            "#,
            namespace_id
        )
        .execute(&self.pool)
        .await
        .map_err(StoreError::from)?;

        Ok(result.rows_affected() > 0)
    }

    async fn soft_delete(&self, namespace_id: &str) -> Result<bool, StoreError> {
        let now = Utc::now();
        let result = sqlx::query!(
            r#"
            UPDATE kagzi.namespaces
            SET deleted_at = $2, updated_at = $2
            WHERE namespace_id = $1 AND deleted_at IS NULL
            "#,
            namespace_id,
            now
        )
        .execute(&self.pool)
        .await
        .map_err(StoreError::from)?;

        Ok(result.rows_affected() > 0)
    }

    async fn count_resources(&self, namespace_id: &str) -> Result<ResourceCount, StoreError> {
        // Count workflows (excluding terminal states)
        let workflow_count = sqlx::query_scalar!(
            r#"
            SELECT COUNT(*) as "count!"
            FROM kagzi.workflow_runs
            WHERE namespace_id = $1
              AND status NOT IN ('COMPLETED', 'FAILED', 'CANCELLED')
            "#,
            namespace_id
        )
        .fetch_one(&self.pool)
        .await
        .map_err(StoreError::from)?;

        // Count workers (excluding offline)
        let worker_count = sqlx::query_scalar!(
            r#"
            SELECT COUNT(*) as "count!"
            FROM kagzi.workers
            WHERE namespace_id = $1
              AND status != 'OFFLINE'
            "#,
            namespace_id
        )
        .fetch_one(&self.pool)
        .await
        .map_err(StoreError::from)?;

        // Count schedules (workflows with schedule_id that are in SCHEDULED status)
        let schedule_count = sqlx::query_scalar!(
            r#"
            SELECT COUNT(*) as "count!"
            FROM kagzi.workflow_runs
            WHERE namespace_id = $1
              AND status = 'SCHEDULED'
              AND schedule_id IS NOT NULL
            "#,
            namespace_id
        )
        .fetch_one(&self.pool)
        .await
        .map_err(StoreError::from)?;

        Ok(ResourceCount {
            workflows: workflow_count,
            workers: worker_count,
            schedules: schedule_count,
        })
    }

    async fn list_distinct_namespaces(&self) -> Result<Vec<String>, StoreError> {
        let namespaces = sqlx::query_scalar!(
            r#"
            SELECT DISTINCT namespace_id
            FROM kagzi.namespaces
            WHERE deleted_at IS NULL
            ORDER BY namespace_id ASC
            "#
        )
        .fetch_all(&self.pool)
        .await
        .map_err(StoreError::from)?;

        Ok(namespaces)
    }

    async fn get_or_create(&self, namespace_id: &str) -> Result<Namespace, StoreError> {
        // First try to find existing namespace
        if let Some(namespace) = self.find_by_id(namespace_id, false).await? {
            return Ok(namespace);
        }

        // Namespace doesn't exist, create it
        let now = Utc::now();
        let namespace = sqlx::query_as!(
            Namespace,
            r#"
            INSERT INTO kagzi.namespaces (namespace_id, display_name, description, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $4)
            ON CONFLICT (namespace_id) DO UPDATE
            SET updated_at = $4
            RETURNING namespace_id, display_name, description, created_at, updated_at, deleted_at
            "#,
            namespace_id,
            namespace_id, // Use namespace_id as display_name for auto-created namespaces
            Option::<String>::None, // No description for auto-created namespaces
            now
        )
        .fetch_one(&self.pool)
        .await
        .map_err(StoreError::from)?;

        Ok(namespace)
    }
}
