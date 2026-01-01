# Schedule Consolidation Plan (MVP)

## Philosophy

A schedule is just a workflow that knows when to create copies of itself.

---

## Final Schema: 6 → 4 Tables

```
┌─────────────────────────────────────────────────────────────────────────────┐
│  BEFORE (6 tables)              │  AFTER (4 tables)                        │
├─────────────────────────────────┼──────────────────────────────────────────┤
│  workflow_runs                  │  workflow_runs (+ 2 columns)             │
│  workflow_payloads              │  workflow_payloads                        │
│  step_runs                      │  step_runs                                │
│  workers                        │  workers (- 4 columns)                    │
│  schedules                      │  ─ DELETED ─                              │
│  schedule_firings               │  ─ DELETED ─                              │
└─────────────────────────────────┴──────────────────────────────────────────┘

New columns: cron_expr, schedule_id
Removed columns: max_concurrent, active_count, total_completed, total_failed
```

---

## Decisions

| Question           | Decision                                    |
| ------------------ | ------------------------------------------- |
| Timezone           | Server timezone                             |
| Deletion           | Keep fired instances (`ON DELETE SET NULL`) |
| Multiple schedules | Allowed (no unique constraint)              |
| Deduplication      | Unique on `(schedule_id, available_at)`     |
| Resume from pause  | Skip missed, wait for next occurrence       |
| Natural language   | Deferred (cron only for MVP)                |
| Catchup            | Deferred (always skip to next)              |

---

## 1. Schema Changes

### workflow_runs (Add 2 Columns)

```sql
ALTER TABLE kagzi.workflow_runs ADD COLUMN cron_expr TEXT;
ALTER TABLE kagzi.workflow_runs ADD COLUMN schedule_id UUID;
```

| Column        | Type | Purpose                            |
| ------------- | ---- | ---------------------------------- |
| `cron_expr`   | TEXT | Cron expression (templates only)   |
| `schedule_id` | UUID | Links fired runs to their template |

### New Status Variants

```sql
-- Extend status to include:
-- SCHEDULED: Active schedule template
-- PAUSED: Disabled schedule template
```

### How It Works

| Row Type          | cron_expr        | schedule_id | status    | available_at   |
| ----------------- | ---------------- | ----------- | --------- | -------------- |
| Schedule template | `"0 17 * * FRI"` | NULL        | SCHEDULED | Next fire time |
| Fired instance    | NULL             | → template  | PENDING   | Fire time      |
| Normal workflow   | NULL             | NULL        | PENDING   | Created time   |
| Paused schedule   | `"0 12 * * *"`   | NULL        | PAUSED    | NULL           |

### workers (Drop 4 Columns)

```sql
ALTER TABLE kagzi.workers DROP COLUMN max_concurrent;
ALTER TABLE kagzi.workers DROP COLUMN active_count;
ALTER TABLE kagzi.workers DROP COLUMN total_completed;
ALTER TABLE kagzi.workers DROP COLUMN total_failed;
```

### Drop Old Tables

```sql
DROP TABLE kagzi.schedule_firings;
DROP TABLE kagzi.schedules;
```

---

## 2. Indexes

```sql
-- Find due schedules
CREATE INDEX idx_workflow_schedules_due
ON kagzi.workflow_runs (namespace_id, available_at)
WHERE status = 'SCHEDULED';

-- Schedule firing history
CREATE INDEX idx_workflow_schedule_history
ON kagzi.workflow_runs (schedule_id, created_at DESC)
WHERE schedule_id IS NOT NULL;

-- Deduplication: prevent double-firing
CREATE UNIQUE INDEX idx_schedule_firing_unique
ON kagzi.workflow_runs (schedule_id, available_at)
WHERE schedule_id IS NOT NULL;

-- Foreign key
ALTER TABLE kagzi.workflow_runs
ADD CONSTRAINT fk_schedule_id
FOREIGN KEY (schedule_id) REFERENCES kagzi.workflow_runs(run_id) ON DELETE SET NULL;
```

---

## 3. Model Changes

### WorkflowStatus

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowStatus {
    Pending,
    Running,
    Sleeping,
    Completed,
    Failed,
    Cancelled,
    Scheduled,  // NEW: Active schedule template
    Paused,     // NEW: Disabled schedule template
}

impl WorkflowStatus {
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }

    pub fn is_schedule_template(&self) -> bool {
        matches!(self, Self::Scheduled | Self::Paused)
    }
}
```

### WorkflowRun

```rust
#[derive(Debug, Clone)]
pub struct WorkflowRun {
    // ... existing fields ...

    pub cron_expr: Option<String>,      // Cron expression (templates only)
    pub schedule_id: Option<Uuid>,      // Links to template (fired instances)
}
```

---

## 4. Repository Changes

### New Methods

```rust
impl WorkflowRepository {
    /// Find schedule templates due to fire.
    async fn find_due_schedules(
        &self,
        namespace_id: &str,
        now: DateTime<Utc>,
        limit: i64,
    ) -> Result<Vec<WorkflowRun>, StoreError>;

    /// Create fired instance from template (idempotent).
    /// Returns None if already fired for this time.
    async fn create_schedule_instance(
        &self,
        template_run_id: Uuid,
        fire_at: DateTime<Utc>,
    ) -> Result<Option<Uuid>, StoreError>;

    /// Update schedule's next fire time.
    async fn update_next_fire(
        &self,
        run_id: Uuid,
        next_fire_at: DateTime<Utc>,
    ) -> Result<(), StoreError>;
}
```

### Query: Find Due Schedules

```sql
SELECT * FROM kagzi.workflow_runs
WHERE namespace_id = $1
  AND status = 'SCHEDULED'
  AND available_at <= $2
ORDER BY available_at
LIMIT $3;
```

### Query: Create Instance (Idempotent)

```sql
WITH template AS (
    SELECT * FROM kagzi.workflow_runs WHERE run_id = $1
),
payload AS (
    SELECT input FROM kagzi.workflow_payloads WHERE run_id = $1
),
inserted AS (
    INSERT INTO kagzi.workflow_runs (
        run_id, namespace_id, external_id, task_queue, workflow_type,
        status, available_at, schedule_id, version, retry_policy
    )
    SELECT
        $2,                          -- new run_id
        t.namespace_id,
        $3,                          -- generated external_id
        t.task_queue,
        t.workflow_type,
        'PENDING',
        $4,                          -- fire_at
        t.run_id,                    -- schedule_id = template
        t.version,
        t.retry_policy
    FROM template t
    ON CONFLICT (schedule_id, available_at)
        WHERE schedule_id IS NOT NULL
        DO NOTHING
    RETURNING run_id
)
INSERT INTO kagzi.workflow_payloads (run_id, input)
SELECT i.run_id, p.input
FROM inserted i, payload p
RETURNING run_id;
```

---

## 5. Coordinator (Simplified)

```rust
async fn fire_due_schedules(
    store: &PgStore,
    queue: &impl QueueNotifier,
    now: DateTime<Utc>,
    batch_size: i64,
) -> Result<u32, StoreError> {
    let templates = store
        .workflows()
        .find_due_schedules("default", now, batch_size)
        .await?;

    let mut fired = 0;

    for template in templates {
        // Try to fire (idempotent - will skip if already fired)
        if let Some(run_id) = store
            .workflows()
            .create_schedule_instance(template.run_id, template.available_at.unwrap())
            .await?
        {
            info!(
                schedule_id = %template.run_id,
                run_id = %run_id,
                "Fired schedule"
            );
            queue.notify(&template.namespace_id, &template.task_queue).await?;
            fired += 1;
        }

        // Update to next occurrence (always skip missed)
        let cron = cron::Schedule::from_str(template.cron_expr.as_ref().unwrap())?;
        let next_fire = cron
            .after(&now)
            .next()
            .unwrap_or(now + chrono::Duration::days(365));

        store
            .workflows()
            .update_next_fire(template.run_id, next_fire)
            .await?;
    }

    Ok(fired)
}
```

---

## 6. Service Changes

### CreateSchedule

```rust
async fn create_schedule(&self, req: CreateScheduleRequest) -> Result<...> {
    // Validate cron expression
    let cron = cron::Schedule::from_str(&req.cron_expr)
        .map_err(|e| Status::invalid_argument(format!("Invalid cron: {e}")))?;

    let first_fire = cron.upcoming(Utc).next()
        .ok_or_else(|| Status::invalid_argument("Cron has no future occurrences"))?;

    let run_id = Uuid::now_v7();

    // Create template as workflow_run with status=SCHEDULED
    self.store.workflows().create(CreateWorkflow {
        run_id,
        namespace_id: req.namespace_id,
        external_id: format!("schedule-{}", run_id),
        task_queue: req.task_queue,
        workflow_type: req.workflow_type,
        status: WorkflowStatus::Scheduled,
        cron_expr: Some(req.cron_expr),
        available_at: Some(first_fire),
        input: req.input,
        ..Default::default()
    }).await?;

    Ok(Response::new(CreateScheduleResponse {
        schedule_id: run_id.to_string(),
    }))
}
```

### GetSchedule / ListSchedules

```rust
// GetSchedule: find by run_id, verify cron_expr is set
// ListSchedules: filter WHERE status IN ('SCHEDULED', 'PAUSED')
```

### UpdateSchedule (Pause/Resume)

```rust
async fn update_schedule(&self, req: UpdateScheduleRequest) -> Result<...> {
    let run_id = Uuid::parse_str(&req.schedule_id)?;

    let new_status = match req.enabled {
        Some(true) => Some(WorkflowStatus::Scheduled),
        Some(false) => Some(WorkflowStatus::Paused),
        None => None,
    };

    // On resume: calculate next fire from NOW (skip missed)
    let next_fire = if new_status == Some(WorkflowStatus::Scheduled) {
        let template = self.store.workflows().find_by_id(run_id).await?;
        let cron = cron::Schedule::from_str(&template.cron_expr.unwrap())?;
        Some(cron.after(&Utc::now()).next().unwrap())
    } else {
        None
    };

    self.store.workflows().update(run_id, UpdateWorkflow {
        status: new_status,
        available_at: next_fire,
        ..Default::default()
    }).await?;

    Ok(Response::new(Empty {}))
}
```

### DeleteSchedule

```rust
async fn delete_schedule(&self, req: DeleteScheduleRequest) -> Result<...> {
    let run_id = Uuid::parse_str(&req.schedule_id)?;

    // Delete template; fired instances become orphaned (schedule_id = NULL)
    self.store.workflows().delete(run_id).await?;

    Ok(Response::new(Empty {}))
}
```

### GetScheduleHistory

```rust
// Query fired instances: WHERE schedule_id = $1 ORDER BY created_at DESC
let history = self.store.workflows()
    .list(ListWorkflowsParams {
        schedule_id: Some(template_run_id),
        ..Default::default()
    })
    .await?;
```

---

## 7. Migrations

### Migration 1: Add Schedule Columns

```sql
-- migrations/20260102000001_add_schedule_columns.sql

-- Add schedule columns
ALTER TABLE kagzi.workflow_runs ADD COLUMN cron_expr TEXT;
ALTER TABLE kagzi.workflow_runs ADD COLUMN schedule_id UUID;

-- Index: find due schedules
CREATE INDEX idx_workflow_schedules_due
ON kagzi.workflow_runs (namespace_id, available_at)
WHERE status = 'SCHEDULED';

-- Index: schedule history
CREATE INDEX idx_workflow_schedule_history
ON kagzi.workflow_runs (schedule_id, created_at DESC)
WHERE schedule_id IS NOT NULL;

-- Index: deduplication (prevent double-firing)
CREATE UNIQUE INDEX idx_schedule_firing_unique
ON kagzi.workflow_runs (schedule_id, available_at)
WHERE schedule_id IS NOT NULL;

-- Foreign key (self-referential)
ALTER TABLE kagzi.workflow_runs
ADD CONSTRAINT fk_schedule_id
FOREIGN KEY (schedule_id) REFERENCES kagzi.workflow_runs(run_id) ON DELETE SET NULL;
```

### Migration 2: Drop Schedule Tables

```sql
-- migrations/20260102000002_drop_schedule_tables.sql

DROP TABLE IF EXISTS kagzi.schedule_firings;
DROP TABLE IF EXISTS kagzi.schedules;
```

### Migration 3: Slim Workers

```sql
-- migrations/20260102000003_slim_workers.sql

ALTER TABLE kagzi.workers DROP COLUMN IF EXISTS max_concurrent;
ALTER TABLE kagzi.workers DROP COLUMN IF EXISTS active_count;
ALTER TABLE kagzi.workers DROP COLUMN IF EXISTS total_completed;
ALTER TABLE kagzi.workers DROP COLUMN IF EXISTS total_failed;
```

---

## 8. Files to Modify

| Action     | File                                                     |
| ---------- | -------------------------------------------------------- |
| **Modify** | `crates/kagzi-store/src/models/workflow.rs`              |
| **Modify** | `crates/kagzi-store/src/models/mod.rs`                   |
| **Modify** | `crates/kagzi-store/src/postgres/workflow/mod.rs`        |
| **Modify** | `crates/kagzi-store/src/repository/workflow.rs`          |
| **Modify** | `crates/kagzi-store/src/lib.rs`                          |
| **Modify** | `crates/kagzi-server/src/coordinator.rs`                 |
| **Modify** | `crates/kagzi-server/src/workflow_schedule_service.rs`   |
| **Modify** | `crates/kagzi-server/src/proto_convert.rs`               |
| **Modify** | `ARCHITECTURE.md`                                        |
| **Delete** | `crates/kagzi-store/src/models/workflow_schedule.rs`     |
| **Delete** | `crates/kagzi-store/src/postgres/workflow_schedule.rs`   |
| **Delete** | `crates/kagzi-store/src/repository/workflow_schedule.rs` |

---

## 9. Deferred to v2

| Feature                                    | Reason                                 |
| ------------------------------------------ | -------------------------------------- |
| Natural language (`"every friday at 5pm"`) | Add `schedule_expr` column later       |
| Catchup (`max_catchup`)                    | Add column + loop logic later          |
| Timezone per schedule                      | Add `timezone` column if users request |

---

## 10. Summary

| Metric           | Value |
| ---------------- | ----- |
| Tables           | 6 → 4 |
| New columns      | 2     |
| Dropped columns  | 4     |
| Files to modify  | 9     |
| Files to delete  | 3     |
| New dependencies | 0     |
| Estimated LOC    | ~300  |
