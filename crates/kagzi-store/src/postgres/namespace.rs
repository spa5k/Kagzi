use async_trait::async_trait;
use chrono::Utc;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::error::StoreError;
use crate::models::{
    CreateNamespace, Namespace, NamespaceStats, PaginatedResult, UpdateNamespace,
    WorkflowStatusCount, WorkflowTypeCount, WorkflowTypeInfo,
};
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
            INSERT INTO kagzi.namespaces (namespace, display_name, description, enabled, created_at, updated_at)
            VALUES ($1, $2, $3, TRUE, $4, $4)
            RETURNING id, namespace, display_name, description, enabled, created_at, updated_at
            "#,
            params.namespace,
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
                    message: format!("Namespace '{}' already exists", params.namespace),
                };
            }
            StoreError::from(e)
        })?;

        Ok(namespace)
    }

    async fn find_by_id(&self, namespace: &str) -> Result<Option<Namespace>, StoreError> {
        sqlx::query_as!(
            Namespace,
            r#"
            SELECT id, namespace, display_name, description, enabled, created_at, updated_at
            FROM kagzi.namespaces
            WHERE namespace = $1
            "#,
            namespace
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(StoreError::from)
    }

    async fn find_by_internal_id(&self, id: Uuid) -> Result<Option<Namespace>, StoreError> {
        sqlx::query_as!(
            Namespace,
            r#"
            SELECT id, namespace, display_name, description, enabled, created_at, updated_at
            FROM kagzi.namespaces
            WHERE id = $1
            "#,
            id
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(StoreError::from)
    }

    async fn list(
        &self,
        page_size: i32,
        cursor: Option<String>,
    ) -> Result<PaginatedResult<Namespace, String>, StoreError> {
        let limit = page_size.clamp(1, 100);

        let namespaces = if let Some(cursor_id) = cursor {
            sqlx::query_as!(
                Namespace,
                r#"
                SELECT id, namespace, display_name, description, enabled, created_at, updated_at
                FROM kagzi.namespaces
                WHERE namespace > $1
                ORDER BY namespace ASC
                LIMIT $2
                "#,
                cursor_id,
                limit as i64
            )
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as!(
                Namespace,
                r#"
                SELECT id, namespace, display_name, description, enabled, created_at, updated_at
                FROM kagzi.namespaces
                ORDER BY namespace ASC
                LIMIT $1
                "#,
                limit as i64
            )
            .fetch_all(&self.pool)
            .await?
        };

        let has_more = namespaces.len() == limit as usize;
        let next_cursor = namespaces.last().map(|n| n.namespace.clone());

        Ok(PaginatedResult {
            items: namespaces,
            next_cursor,
            has_more,
        })
    }

    async fn update(
        &self,
        namespace: &str,
        params: UpdateNamespace,
    ) -> Result<Namespace, StoreError> {
        let now = Utc::now();

        let ns = match (params.display_name, params.description) {
            (Some(name), Some(desc)) => {
                sqlx::query_as!(
                    Namespace,
                    r#"
                    UPDATE kagzi.namespaces
                    SET display_name = $2, description = $3, updated_at = $4
                    WHERE namespace = $1
                    RETURNING id, namespace, display_name, description, enabled, created_at, updated_at
                    "#,
                    namespace,
                    name,
                    desc,
                    now
                )
                .fetch_optional(&self.pool)
                .await?
            }
            (Some(name), None) => {
                sqlx::query_as!(
                    Namespace,
                    r#"
                    UPDATE kagzi.namespaces
                    SET display_name = $2, updated_at = $3
                    WHERE namespace = $1
                    RETURNING id, namespace, display_name, description, enabled, created_at, updated_at
                    "#,
                    namespace,
                    name,
                    now
                )
                .fetch_optional(&self.pool)
                .await?
            }
            (None, Some(desc)) => {
                sqlx::query_as!(
                    Namespace,
                    r#"
                    UPDATE kagzi.namespaces
                    SET description = $2, updated_at = $3
                    WHERE namespace = $1
                    RETURNING id, namespace, display_name, description, enabled, created_at, updated_at
                    "#,
                    namespace,
                    desc,
                    now
                )
                .fetch_optional(&self.pool)
                .await?
            }
            (None, None) => {
                // No fields to update, just return existing
                return self
                    .find_by_id(namespace)
                    .await?
                    .ok_or_else(|| StoreError::not_found("namespace", namespace));
            }
        };

        ns.ok_or_else(|| StoreError::not_found("namespace", namespace))
    }

    async fn enable(&self, namespace: &str) -> Result<bool, StoreError> {
        let now = Utc::now();
        let result = sqlx::query!(
            r#"
            UPDATE kagzi.namespaces
            SET enabled = TRUE, updated_at = $2
            WHERE namespace = $1
            "#,
            namespace,
            now
        )
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }

    async fn disable(&self, namespace: &str) -> Result<bool, StoreError> {
        let now = Utc::now();
        let result = sqlx::query!(
            r#"
            UPDATE kagzi.namespaces
            SET enabled = FALSE, updated_at = $2
            WHERE namespace = $1
            "#,
            namespace,
            now
        )
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }

    async fn get_stats(&self, namespace: &str) -> Result<NamespaceStats, StoreError> {
        // Get workflow counts by status in a single query
        let status_rows = sqlx::query!(
            r#"
            SELECT status, COUNT(*) as count
            FROM kagzi.workflow_runs
            WHERE namespace = $1
            GROUP BY status
            "#,
            namespace
        )
        .fetch_all(&self.pool)
        .await?;

        let workflows_by_status: Vec<WorkflowStatusCount> = status_rows
            .iter()
            .map(|row| WorkflowStatusCount {
                status: row.status.clone(),
                count: row.count.unwrap_or(0),
            })
            .collect();

        // Get workflow counts by type
        let type_rows = sqlx::query!(
            r#"
            SELECT
                workflow_type,
                COUNT(*) as total_runs,
                COUNT(*) FILTER (
                    WHERE status NOT IN ('COMPLETED', 'FAILED', 'CANCELLED')
                ) as active_runs,
                ARRAY_AGG(DISTINCT task_queue) FILTER (WHERE task_queue IS NOT NULL) as task_queues
            FROM kagzi.workflow_runs
            WHERE namespace = $1
            GROUP BY workflow_type
            ORDER BY workflow_type
            "#,
            namespace
        )
        .fetch_all(&self.pool)
        .await?;

        let workflows_by_type: Vec<WorkflowTypeCount> = type_rows
            .iter()
            .map(|row| WorkflowTypeCount {
                workflow_type: row.workflow_type.clone(),
                total_runs: row.total_runs.unwrap_or(0),
                active_runs: row.active_runs.unwrap_or(0),
                task_queues: row.task_queues.clone().unwrap_or_default(),
            })
            .collect();

        // Calculate totals from the grouped data
        let total_workflows: i64 = workflows_by_type.iter().map(|t| t.total_runs).sum();
        let pending_workflows = workflows_by_status
            .iter()
            .find(|s| s.status == "PENDING")
            .map(|s| s.count)
            .unwrap_or(0);
        let running_workflows = workflows_by_status
            .iter()
            .find(|s| s.status == "RUNNING")
            .map(|s| s.count)
            .unwrap_or(0);
        let completed_workflows = workflows_by_status
            .iter()
            .find(|s| s.status == "COMPLETED")
            .map(|s| s.count)
            .unwrap_or(0);
        let failed_workflows = workflows_by_status
            .iter()
            .find(|s| s.status == "FAILED")
            .map(|s| s.count)
            .unwrap_or(0);

        // Get worker counts
        let worker_counts = sqlx::query!(
            r#"
            SELECT
                COUNT(*) FILTER (WHERE status != 'OFFLINE') as total,
                COUNT(*) FILTER (WHERE status = 'ONLINE') as online
            FROM kagzi.workers
            WHERE namespace = $1
            "#,
            namespace
        )
        .fetch_one(&self.pool)
        .await?;

        let total_workers = worker_counts.total.unwrap_or(0);
        let online_workers = worker_counts.online.unwrap_or(0);

        // Get schedule counts (scheduled workflows with cron expression)
        let schedule_counts = sqlx::query!(
            r#"
            SELECT
                COUNT(*) as total,
                COUNT(*) as enabled
            FROM kagzi.workflow_runs
            WHERE namespace = $1
              AND status = 'SCHEDULED'
              AND schedule_id IS NOT NULL
            "#,
            namespace
        )
        .fetch_one(&self.pool)
        .await?;

        let total_schedules = schedule_counts.total.unwrap_or(0);
        let enabled_schedules = schedule_counts.enabled.unwrap_or(0);

        Ok(NamespaceStats {
            total_workflows,
            pending_workflows,
            running_workflows,
            completed_workflows,
            failed_workflows,
            total_workers,
            online_workers,
            total_schedules,
            enabled_schedules,
            workflows_by_status,
            workflows_by_type,
        })
    }

    async fn list_workflow_types(
        &self,
        namespace: &str,
        page_size: i32,
        cursor: Option<String>,
    ) -> Result<PaginatedResult<WorkflowTypeInfo, String>, StoreError> {
        let limit = page_size.clamp(1, 100);

        // Use dynamic query to avoid type issues with sqlx macro
        let query_str = if cursor.is_some() {
            r#"
            SELECT
                workflow_type,
                COUNT(*) as total_runs,
                COUNT(*) FILTER (
                    WHERE status NOT IN ('COMPLETED', 'FAILED', 'CANCELLED')
                ) as active_runs
            FROM kagzi.workflow_runs
            WHERE namespace = $1 AND workflow_type > $2
            GROUP BY workflow_type
            ORDER BY workflow_type ASC
            LIMIT $3
            "#
        } else {
            r#"
            SELECT
                workflow_type,
                COUNT(*) as total_runs,
                COUNT(*) FILTER (
                    WHERE status NOT IN ('COMPLETED', 'FAILED', 'CANCELLED')
                ) as active_runs
            FROM kagzi.workflow_runs
            WHERE namespace = $1
            GROUP BY workflow_type
            ORDER BY workflow_type ASC
            LIMIT $2
            "#
        };

        let rows = if let Some(cursor_type) = cursor {
            sqlx::query(query_str)
                .bind(namespace)
                .bind(cursor_type)
                .bind(limit as i64)
                .fetch_all(&self.pool)
                .await?
        } else {
            sqlx::query(query_str)
                .bind(namespace)
                .bind(limit as i64)
                .fetch_all(&self.pool)
                .await?
        };

        // For each workflow type, get the task queues separately
        let mut items = Vec::new();
        for row in rows {
            let workflow_type: String = row.get("workflow_type");
            let total_runs: i64 = row.get("total_runs");
            let active_runs: i64 = row.get("active_runs");

            // Get task queues for this workflow type
            let queue_rows = sqlx::query!(
                r#"
                SELECT DISTINCT task_queue
                FROM kagzi.workflow_runs
                WHERE namespace = $1 AND workflow_type = $2
                AND task_queue IS NOT NULL
                LIMIT 50
                "#,
                namespace,
                workflow_type
            )
            .fetch_all(&self.pool)
            .await?;

            let task_queues: Vec<String> =
                queue_rows.iter().map(|r| r.task_queue.clone()).collect();

            items.push(WorkflowTypeInfo {
                workflow_type,
                total_runs,
                active_runs,
                task_queues,
            });
        }

        let has_more = items.len() == limit as usize;
        let next_cursor = items.last().map(|t| t.workflow_type.clone());

        Ok(PaginatedResult {
            items,
            next_cursor,
            has_more,
        })
    }

    async fn list_distinct_namespaces(&self) -> Result<Vec<String>, StoreError> {
        let namespaces = sqlx::query_scalar!(
            r#"
            SELECT DISTINCT namespace
            FROM kagzi.namespaces
            ORDER BY namespace ASC
            "#
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(namespaces)
    }

    async fn get_or_create(&self, namespace: &str) -> Result<Namespace, StoreError> {
        // First try to find existing namespace
        if let Some(ns) = self.find_by_id(namespace).await? {
            return Ok(ns);
        }

        // Namespace doesn't exist, create it
        let now = Utc::now();
        let ns = sqlx::query_as!(
            Namespace,
            r#"
            INSERT INTO kagzi.namespaces (namespace, display_name, description, enabled, created_at, updated_at)
            VALUES ($1, $2, NULL, TRUE, $3, $3)
            ON CONFLICT (namespace) DO UPDATE
            SET updated_at = $3
            RETURNING id, namespace, display_name, description, enabled, created_at, updated_at
            "#,
            namespace,
            namespace, // Use namespace as display_name for auto-created namespaces
            now
        )
        .fetch_one(&self.pool)
        .await?;

        Ok(ns)
    }
}
