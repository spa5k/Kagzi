---
name: River Pattern Refactoring
overview: Implement payload size limits (2MB max, 1MB warning), configurable watchdog batch size, opaque binary payload support, and refactor to the River Pattern architecture with batching and event-driven work distribution.
todos:
  - id: pr1-payload-limits
    content: "PR 1: Add payload size constants, validation in step.rs, update README"
    status: completed
  - id: pr1-binary-support
    content: "PR 1: Remove JSON validation from helpers.rs, update models to Vec<u8>"
    status: completed
  - id: pr2-watchdog-config
    content: "PR 2: Add wake_sleeping_batch_size to WatchdogSettings and use in watchdog.rs"
    status: completed
  - id: pr3-migrations
    content: "PR 3: Add migrations for partial indexes and NOTIFY trigger"
    status: completed
  - id: pr4-repo-trait
    content: "PR 4: Add claim_workflow_batch and wait_for_new_work to WorkflowRepository trait"
    status: completed
  - id: pr4-pg-impl
    content: "PR 4: Implement batch claim and PgListener-based wait in Postgres"
    status: completed
  - id: pr5-distributor
    content: "PR 5: Rewrite WorkDistributor to remove cache, use batch claim + notify"
    status: completed
  - id: pr6-counters
    content: "PR 6: Remove counter increment from claim hot path, rely on reconciliation"
    status: completed
---

# River Pattern Refactoring & Payload Improvements

## PR 1: Payload Size Limits & Binary Support

### 1.1 Add PayloadSettings to Config

**File:** `crates/kagzi-server/src/config.rs`

```rust
#[derive(Debug, Clone, Deserialize)]
pub struct PayloadSettings {
    #[serde(default = "default_payload_warn_threshold_bytes")]
    pub warn_threshold_bytes: usize,
    #[serde(default = "default_payload_max_size_bytes")]
    pub max_size_bytes: usize,
}

fn default_payload_warn_threshold_bytes() -> usize {
    1024 * 1024  // 1MB
}

fn default_payload_max_size_bytes() -> usize {
    2 * 1024 * 1024  // 2MB
}

// Add to Settings struct:
pub struct Settings {
    // ... existing fields ...
    pub payload: PayloadSettings,
}
```

Configurable via: `KAGZI__PAYLOAD__MAX_SIZE_BYTES`, `KAGZI__PAYLOAD__WARN_THRESHOLD_BYTES`

### 1.2 Pass PayloadSettings to PgStore

**File:** `crates/kagzi-store/src/postgres/mod.rs`

```rust
#[derive(Clone)]
pub struct StoreConfig {
    pub payload_warn_threshold_bytes: usize,
    pub payload_max_size_bytes: usize,
}

#[derive(Clone)]
pub struct PgStore {
    pool: PgPool,
    config: StoreConfig,
}

impl PgStore {
    pub fn new(pool: PgPool, config: StoreConfig) -> Self {
        Self { pool, config }
    }

    pub fn steps(&self) -> PgStepRepository {
        PgStepRepository::new(self.pool.clone(), self.config.clone())
    }
    // ... other methods unchanged
}
```

**File:** `crates/kagzi-server/src/main.rs`

```rust
let store_config = kagzi_store::StoreConfig {
    payload_warn_threshold_bytes: settings.payload.warn_threshold_bytes,
    payload_max_size_bytes: settings.payload.max_size_bytes,
};
let store = PgStore::new(pool, store_config);
```

### 1.3 Update Step Repository with Config-Based Validation

**File:** `crates/kagzi-store/src/postgres/step.rs`

```rust
pub struct PgStepRepository {
    pool: PgPool,
    config: StoreConfig,
}

impl PgStepRepository {
    pub fn new(pool: PgPool, config: StoreConfig) -> Self {
        Self { pool, config }
    }

    fn validate_payload_size(&self, data: &[u8], context: &str) -> Result<(), StoreError> {
        let size = data.len();
        if size > self.config.payload_max_size_bytes {
            return Err(StoreError::invalid_argument(format!(
                "{} exceeds maximum size of {} bytes ({} bytes). Do not use Kagzi for blob storage.",
                context, self.config.payload_max_size_bytes, size
            )));
        }
        if size > self.config.payload_warn_threshold_bytes {
            warn!(size_bytes = size, context = context,
                  "Payload exceeds {} bytes. Consider storing large data externally.",
                  self.config.payload_warn_threshold_bytes);
        }
        Ok(())
    }
}

// Use in complete(), begin(), etc.
```

### 1.3 Remove JSON Validation from Server

**File:** `crates/kagzi-server/src/helpers.rs`

Replace `payload_to_json()` with opaque handling:

```rust
pub fn payload_to_bytes(payload: Option<Payload>) -> Vec<u8> {
    payload.map(|p| p.data).unwrap_or_default()
}

pub fn validate_payload_size(data: &[u8]) -> Result<(), Status> {
    if data.len() > PAYLOAD_MAX_SIZE_BYTES {
        return Err(invalid_argument(format!(
            "Payload exceeds maximum size of 2MB ({} bytes)",
            data.len()
        )));
    }
    Ok(())
}
```

### 1.4 Update Store Models

**File:** `crates/kagzi-store/src/models.rs`

Change payload fields from `serde_json::Value` to `Vec<u8>`:

```rust
pub struct StepRun {
    // ...
    pub input: Option<Vec<u8>>,  // Was: Option<serde_json::Value>
    pub output: Option<Vec<u8>>, // Was: Option<serde_json::Value>
}
```

### 1.5 Update README

**File:** `README.md`

Add section under "Configuration" or "Best Practices":

```markdown
### Payload Limits

Kagzi enforces payload size limits to prevent database bloat:

- **Warning threshold:** 1MB - Logs a warning for payloads exceeding this size
- **Hard limit:** 2MB - Rejects payloads exceeding this size with an error

**Important:** Kagzi is a workflow orchestration engine, not a blob store. For large data:
- Store blobs in S3/GCS/MinIO and pass references through workflows
- Use external databases for large datasets
- Compress data before passing through Kagzi if necessary
```

---

## PR 2: Configurable Watchdog Batch Size

### 2.1 Update Config

**File:** `crates/kagzi-server/src/config.rs`

```rust
#[derive(Debug, Clone, Deserialize)]
pub struct WatchdogSettings {
    #[serde(default = "default_watchdog_interval_secs")]
    pub interval_secs: u64,
    #[serde(default = "default_worker_stale_threshold_secs")]
    pub worker_stale_threshold_secs: i64,
    #[serde(default = "default_counter_reconcile_interval_secs")]
    pub counter_reconcile_interval_secs: u64,
    #[serde(default = "default_wake_sleeping_batch_size")]
    pub wake_sleeping_batch_size: i32,  // NEW
}

fn default_wake_sleeping_batch_size() -> i32 {
    100
}
```

### 2.2 Update Watchdog

**File:** `crates/kagzi-server/src/watchdog.rs`

Remove constant, use config:

```rust
pub fn spawn(store: PgStore, settings: WatchdogSettings, shutdown: CancellationToken) {
    // ...
    tokio::spawn(run_wake_sleeping(
        store.clone(),
        shutdown.clone(),
        interval,
        settings.wake_sleeping_batch_size,  // From config instead of const
    ));
}
```

---

## PR 3: Database Schema & Partial Indexes

### 3.1 Migration for Partial Index

**File:** `migrations/YYYYMMDD_partial_indexes.sql`

```sql
-- Index for claiming pending/sleeping workflows efficiently
CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_workflow_runs_claimable
ON kagzi.workflow_runs (task_queue, namespace_id, COALESCE(wake_up_at, created_at))
WHERE status IN ('PENDING', 'SLEEPING');

-- Index for step retries
CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_step_runs_pending_retry
ON kagzi.step_runs (retry_at)
WHERE status = 'PENDING' AND retry_at IS NOT NULL;
```

### 3.2 Migration for Postgres NOTIFY Trigger

**File:** `migrations/YYYYMMDD_notify_trigger.sql`

```sql
CREATE OR REPLACE FUNCTION kagzi.notify_new_work() RETURNS trigger AS $$
BEGIN
  PERFORM pg_notify('kagzi_work_' || NEW.namespace_id || '_' || NEW.task_queue, NEW.run_id::text);
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trigger_workflow_new_work
AFTER INSERT OR UPDATE ON kagzi.workflow_runs
FOR EACH ROW
WHEN (NEW.status = 'PENDING' AND (NEW.wake_up_at IS NULL OR NEW.wake_up_at <= NOW()))
EXECUTE FUNCTION kagzi.notify_new_work();
```

---

## PR 4: Repository Abstraction

### 4.1 Update WorkflowRepository Trait

**File:** `crates/kagzi-store/src/repository/workflow.rs`

```rust
#[async_trait]
pub trait WorkflowRepository: Send + Sync {
    // ... existing methods ...

    /// Batch claim multiple workflows atomically
    async fn claim_workflow_batch(
        &self,
        task_queue: &str,
        namespace_id: &str,
        worker_id: &str,
        supported_types: &[String],
        limit: usize,
        lock_duration_secs: i64,
    ) -> Result<Vec<ClaimedWorkflow>, StoreError>;

    /// Wait for new work notification (Postgres: LISTEN, MySQL: short sleep)
    async fn wait_for_new_work(
        &self,
        task_queue: &str,
        namespace_id: &str,
        timeout: Duration,
    ) -> Result<bool, StoreError>;  // Returns true if notified, false on timeout
}
```

### 4.2 Implement Postgres Batch Claim

**File:** `crates/kagzi-store/src/postgres/workflow/queue.rs`

```rust
pub(super) async fn claim_workflow_batch(
    repo: &PgWorkflowRepository,
    task_queue: &str,
    namespace_id: &str,
    worker_id: &str,
    supported_types: &[String],
    limit: usize,
    lock_duration_secs: i64,
) -> Result<Vec<ClaimedWorkflow>, StoreError> {
    // Single query to claim up to `limit` workflows
    let rows = sqlx::query_as!(...)
        .fetch_all(&repo.pool)
        .await?;
    // ...
}
```

### 4.3 Implement Postgres LISTEN/NOTIFY

**File:** `crates/kagzi-store/src/postgres/workflow/notify.rs` (new)

```rust
use sqlx::postgres::PgListener;

pub(super) async fn wait_for_new_work(
    pool: &PgPool,
    task_queue: &str,
    namespace_id: &str,
    timeout: Duration,
) -> Result<bool, StoreError> {
    let channel = format!("kagzi_work_{}_{}", namespace_id, task_queue);
    let mut listener = PgListener::connect_with(pool).await?;
    listener.listen(&channel).await?;

    tokio::select! {
        result = listener.recv() => {
            result?;
            Ok(true)
        }
        _ = tokio::time::sleep(timeout) => Ok(false)
    }
}
```

---

## PR 5: WorkDistributor Refactoring

### 5.1 Rewrite WorkDistributor

**File:** `crates/kagzi-server/src/work_distributor.rs`

Key changes:

1. Remove `cache: Arc<DashMap<..., VecDeque<WorkCandidate>>>`
2. Remove `poll_interval`
3. Implement request-wait-dispatch loop with batch claiming
```rust
async fn run_distribution_loop(self: Arc<Self>, mut request_rx: mpsc::Receiver<WorkRequest>) {
    loop {
        tokio::select! {
            _ = self.shutdown.cancelled() => break,
            Some(request) = request_rx.recv() => {
                // Collect request into pending_requests map
                self.pending_requests.entry(key).or_default().push(request);
            }
        }

        // Process all queues with waiters
        for (namespace_id, task_queue) in self.queues_with_waiters() {
            let waiters = self.take_waiters(&namespace_id, &task_queue);
            let needed = waiters.len();

            // Batch claim from DB
            let claimed = self.store.workflows()
                .claim_workflow_batch(&task_queue, &namespace_id, ..., needed, ...)
                .await?;

            // Distribute to waiters
            for (workflow, waiter) in claimed.into_iter().zip(waiters.iter()) {
                let _ = waiter.respond(workflow.into());
            }

            // Remaining waiters: spawn wait_for_new_work
            if waiters.len() > claimed.len() {
                // Re-queue remaining waiters and wait for notification
                tokio::spawn(async move {
                    let _ = store.workflows()
                        .wait_for_new_work(&task_queue, &namespace_id, Duration::from_secs(5))
                        .await;
                });
            }
        }
    }
}
```


---

## PR 6: Queue Counter Strategy (Deferred Enforcement)

### 6.1 Remove Counter from Claim Hot Path

**File:** `crates/kagzi-store/src/postgres/workflow/queue.rs`

In `claim_workflow_batch`, remove calls to `try_increment_counter_tx`. Instead:

- Worker semaphore provides local concurrency limit
- Watchdog reconciliation provides eventual consistency
- Optional: Add a "soft limit" check that reads counters without locking

### 6.2 Alternative: Async Counter Updates

If strict global limits are needed, update counters asynchronously:

```rust
// After successful claim, fire-and-forget counter update
tokio::spawn(async move {
    let _ = store.workflows().increment_queue_counter(...).await;
});
```

---

## Testing Plan

1. **Unit tests** for payload size validation (both warn and reject paths)
2. **Integration tests** for batch claiming
3. **Integration tests** for LISTEN/NOTIFY wakeup
4. **Load test** with 10k+ timers waking at same second to verify batch size config
5. **Binary payload test** ensuring non-JSON data flows through correctly