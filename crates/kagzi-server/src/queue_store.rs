use std::collections::HashMap;

use chrono::{DateTime, Utc};
use kagzi_store::PgStore;
use serde_json::Value;
use sqlx::Row;

use crate::helpers::{conflict_error, internal_error, invalid_argument_error, not_found_error};

#[derive(Debug, Clone)]
pub struct TaskQueueRow {
    pub namespace: String,
    pub task_queue: String,
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub labels: HashMap<String, String>,
    pub extra: Value,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

fn json_object_to_string_map(value: Value) -> Result<HashMap<String, String>, tonic::Status> {
    match value {
        Value::Object(map) => map
            .into_iter()
            .map(|(k, v)| {
                let s = v.as_str().ok_or_else(|| {
                    invalid_argument_error("labels must be a JSON object with string values")
                })?;
                Ok((k, s.to_string()))
            })
            .collect(),
        Value::Null => Ok(HashMap::new()),
        _ => Err(invalid_argument_error(
            "labels must be a JSON object with string values",
        )),
    }
}

fn parse_extra_json_bytes(bytes: &[u8]) -> Result<Value, tonic::Status> {
    if bytes.is_empty() {
        return Ok(Value::Object(serde_json::Map::new()));
    }
    serde_json::from_slice(bytes)
        .map_err(|e| invalid_argument_error(format!("extra_json must be valid JSON: {e}")))
}

#[derive(Debug)]
pub struct CreateTaskQueueInput {
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub labels: HashMap<String, String>,
    pub extra_json: Vec<u8>,
    pub enabled: bool,
}

pub async fn ensure_task_queue_exists(
    store: &PgStore,
    namespace: &str,
    task_queue: &str,
) -> Result<(), tonic::Status> {
    // Best-effort upsert for implicit creation.
    sqlx::query(
        r#"
        INSERT INTO kagzi.task_queues (namespace, task_queue, display_name, description, labels, extra, enabled)
        VALUES ($1, $2, NULL, NULL, '{}'::jsonb, '{}'::jsonb, TRUE)
        ON CONFLICT (namespace, task_queue) DO NOTHING
        "#,
    )
    .bind(namespace)
    .bind(task_queue)
    .execute(store.pool())
    .await
    .map_err(|e| internal_error(format!("Failed to ensure task queue exists: {e}")))?;
    Ok(())
}

pub async fn create_task_queue(
    store: &PgStore,
    namespace: &str,
    task_queue: &str,
    input: CreateTaskQueueInput,
) -> Result<TaskQueueRow, tonic::Status> {
    let extra = parse_extra_json_bytes(&input.extra_json)?;
    let labels_json = serde_json::to_value(&input.labels)
        .map_err(|e| internal_error(format!("Failed to serialize labels: {e}")))?;

    let row = sqlx::query(
        r#"
        INSERT INTO kagzi.task_queues (namespace, task_queue, display_name, description, labels, extra, enabled)
        VALUES ($1, $2, $3, $4, $5::jsonb, $6::jsonb, $7)
        RETURNING namespace, task_queue, display_name, description, labels, extra, enabled, created_at, updated_at
        "#,
    )
    .bind(namespace)
    .bind(task_queue)
    .bind(input.display_name)
    .bind(input.description)
    .bind(labels_json)
    .bind(extra)
    .bind(input.enabled)
    .fetch_one(store.pool())
    .await
    .map_err(|e| {
        if let sqlx::Error::Database(db_err) = &e {
            if db_err.is_unique_violation() {
                return conflict_error(format!("Queue '{task_queue}' already exists"));
            }
            if db_err.is_foreign_key_violation() {
                return not_found_error(
                    format!("Namespace '{namespace}' not found"),
                    "namespace",
                    namespace,
                );
            }
        }
        internal_error(format!("Failed to create queue: {e}"))
    })?;

    row_to_task_queue_row(row)
}

pub async fn get_task_queue(
    store: &PgStore,
    namespace: &str,
    task_queue: &str,
) -> Result<TaskQueueRow, tonic::Status> {
    let row = sqlx::query(
        r#"
        SELECT namespace, task_queue, display_name, description, labels, extra, enabled, created_at, updated_at
        FROM kagzi.task_queues
        WHERE namespace = $1 AND task_queue = $2
        "#,
    )
    .bind(namespace)
    .bind(task_queue)
    .fetch_optional(store.pool())
    .await
    .map_err(|e| internal_error(format!("Failed to fetch queue: {e}")))?;

    match row {
        None => Err(not_found_error(
            format!("Queue '{task_queue}' not found"),
            "task_queue",
            task_queue,
        )),
        Some(r) => row_to_task_queue_row(r),
    }
}

pub async fn list_task_queues(
    store: &PgStore,
    namespace: &str,
    page_size: i32,
    cursor: Option<String>,
) -> Result<(Vec<TaskQueueRow>, Option<String>, bool), tonic::Status> {
    let page_size = page_size.clamp(1, 200) as i64;
    let limit = page_size + 1;

    let rows = if let Some(cur) = cursor {
        sqlx::query(
            r#"
            SELECT namespace, task_queue, display_name, description, labels, extra, enabled, created_at, updated_at
            FROM kagzi.task_queues
            WHERE namespace = $1 AND task_queue > $2
            ORDER BY task_queue ASC
            LIMIT $3
            "#,
        )
        .bind(namespace)
        .bind(cur)
        .bind(limit)
        .fetch_all(store.pool())
        .await
        .map_err(|e| internal_error(format!("Failed to list queues: {e}")))?
    } else {
        sqlx::query(
            r#"
            SELECT namespace, task_queue, display_name, description, labels, extra, enabled, created_at, updated_at
            FROM kagzi.task_queues
            WHERE namespace = $1
            ORDER BY task_queue ASC
            LIMIT $2
            "#,
        )
        .bind(namespace)
        .bind(limit)
        .fetch_all(store.pool())
        .await
        .map_err(|e| internal_error(format!("Failed to list queues: {e}")))?
    };

    let has_more = rows.len() as i64 > page_size;
    let mut items = Vec::with_capacity(rows.len().min(page_size as usize));
    let mut next_cursor = None;
    for r in rows.into_iter().take(page_size as usize) {
        let item = row_to_task_queue_row(r)?;
        next_cursor = Some(item.task_queue.clone());
        items.push(item);
    }

    Ok((items, next_cursor, has_more))
}

#[derive(Debug, Default)]
pub struct UpdateTaskQueueInput {
    pub display_name: Option<Option<String>>,
    pub description: Option<Option<String>>,
    pub enabled: Option<bool>,
    pub labels: Option<HashMap<String, String>>,
    pub extra_json: Option<Vec<u8>>,
}

pub async fn update_task_queue(
    store: &PgStore,
    namespace: &str,
    task_queue: &str,
    input: UpdateTaskQueueInput,
) -> Result<TaskQueueRow, tonic::Status> {
    let mut builder = sqlx::QueryBuilder::new("UPDATE kagzi.task_queues SET ");
    let mut set = builder.separated(", ");

    if let Some(v) = input.display_name {
        set.push("display_name = ").push_bind(v);
    }
    if let Some(v) = input.description {
        set.push("description = ").push_bind(v);
    }
    if let Some(v) = input.enabled {
        set.push("enabled = ").push_bind(v);
    }
    if let Some(v) = input.labels {
        let labels_json = serde_json::to_value(&v)
            .map_err(|e| internal_error(format!("Failed to serialize labels: {e}")))?;
        set.push("labels = ").push_bind(labels_json);
    }
    if let Some(bytes) = input.extra_json {
        let extra = parse_extra_json_bytes(&bytes)?;
        set.push("extra = ").push_bind(extra);
    }

    set.push("updated_at = NOW()");

    builder
        .push(" WHERE namespace = ")
        .push_bind(namespace)
        .push(" AND task_queue = ")
        .push_bind(task_queue)
        .push(" RETURNING namespace, task_queue, display_name, description, labels, extra, enabled, created_at, updated_at");

    let row = builder
        .build()
        .fetch_optional(store.pool())
        .await
        .map_err(|e| internal_error(format!("Failed to update queue: {e}")))?;

    match row {
        None => Err(not_found_error(
            format!("Queue '{task_queue}' not found"),
            "task_queue",
            task_queue,
        )),
        Some(r) => row_to_task_queue_row(r),
    }
}

fn row_to_task_queue_row(row: sqlx::postgres::PgRow) -> Result<TaskQueueRow, tonic::Status> {
    let namespace: String = row
        .try_get("namespace")
        .map_err(|e| internal_error(format!("Failed to read namespace: {e}")))?;
    let task_queue: String = row
        .try_get("task_queue")
        .map_err(|e| internal_error(format!("Failed to read task_queue: {e}")))?;
    let display_name: Option<String> = row
        .try_get("display_name")
        .map_err(|e| internal_error(format!("Failed to read display_name: {e}")))?;
    let description: Option<String> = row
        .try_get("description")
        .map_err(|e| internal_error(format!("Failed to read description: {e}")))?;
    let labels_value: Value = row
        .try_get("labels")
        .map_err(|e| internal_error(format!("Failed to read labels: {e}")))?;
    let labels = json_object_to_string_map(labels_value)?;
    let extra: Value = row
        .try_get("extra")
        .map_err(|e| internal_error(format!("Failed to read extra: {e}")))?;
    let enabled: bool = row
        .try_get("enabled")
        .map_err(|e| internal_error(format!("Failed to read enabled: {e}")))?;
    let created_at: DateTime<Utc> = row
        .try_get("created_at")
        .map_err(|e| internal_error(format!("Failed to read created_at: {e}")))?;
    let updated_at: DateTime<Utc> = row
        .try_get("updated_at")
        .map_err(|e| internal_error(format!("Failed to read updated_at: {e}")))?;

    Ok(TaskQueueRow {
        namespace,
        task_queue,
        display_name,
        description,
        labels,
        extra,
        enabled,
        created_at,
        updated_at,
    })
}
