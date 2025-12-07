use sqlx::postgres::types::PgHasArrayType;
use sqlx::{Encode, Postgres, QueryBuilder, Type};

/// Column lists reused across repositories to avoid drift.
pub mod columns {
    pub mod workflow {
        pub const BASE: &str = "\
w.run_id, w.namespace_id, w.external_id, w.task_queue, w.workflow_type, \
w.status, w.locked_by, w.attempts, w.error, w.created_at, w.started_at, \
w.finished_at, w.wake_up_at, w.deadline_at, w.version, \
w.parent_step_attempt_id, w.retry_policy";

        pub const WITH_PAYLOAD: &str = "\
w.run_id, w.namespace_id, w.external_id, w.task_queue, w.workflow_type, \
w.status, p.input, p.output, p.context, w.locked_by, w.attempts, w.error, \
w.created_at, w.started_at, w.finished_at, w.wake_up_at, w.deadline_at, \
w.version, w.parent_step_attempt_id, w.retry_policy";
    }

    pub mod step {
        pub const BASE: &str = "\
attempt_id, run_id, step_id, namespace_id, attempt_number, status, \
input, output, error, child_workflow_run_id, created_at, started_at, \
finished_at, retry_at, retry_policy";
    }

    pub mod worker {
        pub const BASE: &str = "\
worker_id, namespace_id, task_queue, status, hostname, pid, version, \
workflow_types, max_concurrent, active_count, total_completed, total_failed, \
registered_at, last_heartbeat_at, deregistered_at, labels";
    }

    pub mod schedule {
        pub const BASE: &str = "\
schedule_id, namespace_id, task_queue, workflow_type, cron_expr, \
input, context, enabled, max_catchup, next_fire_at, last_fired_at, \
version, created_at, updated_at";
    }
}

/// Helper to build SELECT queries with composable filters.
pub struct FilterBuilder<'q> {
    builder: QueryBuilder<'q, Postgres>,
    has_where: bool,
}

impl<'q> FilterBuilder<'q> {
    /// Start a new SELECT statement.
    pub fn select(columns: &str, table: &str) -> Self {
        let mut builder = QueryBuilder::new("SELECT ");
        builder.push(columns).push(" FROM ").push(table);
        Self {
            builder,
            has_where: false,
        }
    }

    /// Append an equality predicate.
    pub fn and_eq<T>(&mut self, column: &str, value: T) -> &mut Self
    where
        T: Encode<'q, Postgres> + Type<Postgres> + 'q,
    {
        self.ensure_where();
        self.builder.push(column).push(" = ").push_bind(value);
        self
    }

    /// Append an optional equality predicate.
    pub fn and_optional_eq<T>(&mut self, column: &str, value: Option<T>) -> &mut Self
    where
        T: Encode<'q, Postgres> + Type<Postgres> + 'q,
    {
        if let Some(v) = value {
            self.and_eq(column, v);
        }
        self
    }

    /// Append an IN predicate if the slice is non-empty.
    pub fn and_in<T>(&mut self, column: &str, values: &'q [T]) -> &mut Self
    where
        T: Encode<'q, Postgres> + Type<Postgres> + PgHasArrayType + 'q,
    {
        if values.is_empty() {
            return self;
        }
        self.ensure_where();
        self.builder
            .push(column)
            .push(" = ANY(")
            .push_bind(values)
            .push(")");
        self
    }

    /// Access the inner builder for final ordering/limits.
    pub fn builder(&mut self) -> &mut QueryBuilder<'q, Postgres> {
        &mut self.builder
    }

    /// Finalize into the underlying QueryBuilder.
    pub fn finalize(self) -> QueryBuilder<'q, Postgres> {
        self.builder
    }

    fn ensure_where(&mut self) {
        if !self.has_where {
            self.builder.push(" WHERE ");
            self.has_where = true;
        } else {
            self.builder.push(" AND ");
        }
    }
}

/// Helper to bind a limit safely.
pub fn push_limit(builder: &mut QueryBuilder<'_, Postgres>, limit: i64) {
    builder.push(" LIMIT ").push_bind(limit);
}

/// Helper to build a cursor tuple predicate `(col1, col2) < (...)`.
pub fn push_tuple_cursor(
    builder: &mut QueryBuilder<'_, Postgres>,
    columns: &[&str],
    predicate: &str,
    args: impl FnOnce(&mut QueryBuilder<'_, Postgres>),
) {
    builder.push(" AND (");
    for (idx, col) in columns.iter().enumerate() {
        if idx > 0 {
            builder.push(", ");
        }
        builder.push(*col);
    }
    builder.push(") ").push(predicate).push(" (");
    args(builder);
    builder.push(")");
}
