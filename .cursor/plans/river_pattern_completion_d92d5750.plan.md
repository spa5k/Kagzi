---
name: River Pattern Completion
overview: "Complete the River Pattern implementation: redesign WorkDistributor with batch claim + LISTEN/NOTIFY, add workflow payload validation (2MB/1MB), fix index migration for online safety, and add focused tests."
todos:
  - id: distributor-batch-notify
    content: "Redesign WorkDistributor: batch claim all waiters, use wait_for_new_work instead of busy-loop respawn"
    status: completed
  - id: workflow-payload-validation
    content: Pass StoreConfig to PgWorkflowRepository, add payload size validation to create/create_batch/complete
    status: completed
  - id: index-migration-fix
    content: Update partial index migration with CONCURRENTLY note or split for production safety
    status: completed
  - id: tests-payload-limits
    content: Add integration tests for payload size rejection (workflow input/output, step input/output)
    status: completed
  - id: tests-batch-notify
    content: Add tests for batch claim behavior and LISTEN/NOTIFY wake-up
    status: completed
---

# River Pattern Completion

## 1. WorkDistributor Redesign (Batch Claim + LISTEN/NOTIFY)

**Problem:** Current implementation claims one workflow per waiter and immediately respawns on empty, creating busy-loop potential. `wait_for_new_work` is never used.

**File:** `crates/kagzi-server/src/work_distributor.rs`

**Changes:**

- Replace sequential per-waiter claiming with batch claim for all waiters at once
- When no work available, call `wait_for_new_work` (LISTEN/NOTIFY) instead of immediate respawn
- Add configurable wait timeout
```rust
// Key changes in process_queue:
async fn process_queue(self: Arc<Self>, key: (String, String)) {
    let requests = { /* take all pending requests */ };
    if requests.is_empty() { return; }
    
    // Group by worker_id + supported_types for batch claim
    let needed = requests.len();
    
    // Single batch claim for all waiters
    let claimed = self.store.workflows()
        .claim_workflow_batch(&key.1, &key.0, &worker_id, &types, needed, LOCK_DURATION)
        .await?;
    
    // Distribute claimed work to waiters
    for (workflow, waiter) in claimed.into_iter().zip(requests.iter()) {
        let _ = waiter.response_tx.send(Some(workflow.into()));
    }
    
    // Remaining waiters: wait for notification instead of busy-loop
    let remaining = requests.into_iter().skip(claimed.len()).collect();
    if !remaining.is_empty() {
        // Re-queue remaining
        self.pending_requests.lock().await.entry(key.clone()).or_default().extend(remaining);
        
        // Wait for NOTIFY or timeout, then retry
        let notified = self.store.workflows()
            .wait_for_new_work(&key.1, &key.0, Duration::from_secs(5))
            .await
            .unwrap_or(false);
        
        if notified || /* timeout */ {
            tokio::spawn(self.clone().process_queue(key));
        }
    }
}
```


**Consideration:** Workers may have different `supported_workflow_types`. For simplicity, batch claim with the union of all types, then match workflows to compatible waiters. More complex: group waiters by type filter first.

---

## 2. Workflow Payload Validation

**Problem:** `create()`, `create_batch()`, and `complete()` write payloads without size checks.

### 2.1 Pass StoreConfig to PgWorkflowRepository

**File:** `crates/kagzi-store/src/postgres/mod.rs`

```rust
pub struct PgWorkflowRepository {
    pool: PgPool,
    config: StoreConfig,  // Add this
}

// Update PgStore::workflows() to pass config
pub fn workflows(&self) -> PgWorkflowRepository {
    PgWorkflowRepository::new(self.pool.clone(), self.config.clone())
}
```

### 2.2 Add validation to workflow state.rs

**File:** `crates/kagzi-store/src/postgres/workflow/state.rs`

```rust
// Add validate_payload_size (same as step.rs)
fn validate_payload_size(config: &StoreConfig, bytes: &[u8], context: &str) -> Result<(), StoreError> {
    // Same logic as PgStepRepository::validate_payload_size
}

// In create():
validate_payload_size(&repo.config, &params.input, "Workflow input")?;

// In create_batch() - for each workflow:
validate_payload_size(&repo.config, &p.input, "Workflow input")?;

// In complete():
validate_payload_size(&repo.config, &output, "Workflow output")?;
```

---

## 3. Safe Index Migration (CONCURRENTLY)

**Problem:** `CREATE INDEX` locks table during build. `CREATE INDEX CONCURRENTLY` cannot run in a transaction.

**File:** `migrations/20251210190500_partial_indexes.sql`

**Options:**

- **A) Use `-- no-transaction` directive** (if sqlx supports it via comment)
- **B) Manual migration script** run outside sqlx with `CONCURRENTLY`
- **C) Document as manual step** for production deployments

**Recommended approach:** Replace migration with documentation comment + manual SQL:

```sql
-- NOTE: For production, run these manually outside a transaction:
-- CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_workflow_runs_claimable ...
-- For dev/test, non-concurrent is acceptable:
CREATE INDEX IF NOT EXISTS idx_workflow_runs_claimable
ON kagzi.workflow_runs (task_queue, namespace_id, COALESCE(wake_up_at, created_at))
WHERE status IN ('PENDING', 'SLEEPING');
```

Or split into a separate `_manual.sql` file with instructions.

---

## 4. Tests

**File:** `crates/tests/tests/integration_tests.rs` (or new test module)

### 4.1 Payload Limit Tests

```rust
#[tokio::test]
async fn test_workflow_input_exceeds_max_size_rejected() {
    let harness = TestHarness::new().await;
    let large_input = vec![0u8; 3 * 1024 * 1024]; // 3MB
    let result = start_workflow_with_bytes(&harness, "test", large_input).await;
    assert!(result.is_err()); // InvalidArgument
}

#[tokio::test]
async fn test_workflow_output_exceeds_max_size_rejected() { ... }

#[tokio::test]
async fn test_step_input_exceeds_max_size_rejected() { ... }
```

### 4.2 Batch Claim Test

```rust
#[tokio::test]
async fn test_batch_claim_returns_multiple_workflows() {
    // Create 5 workflows
    // Batch claim with limit=5
    // Assert 5 returned
}
```

### 4.3 LISTEN/NOTIFY Test

```rust
#[tokio::test]
async fn test_wait_for_new_work_wakes_on_insert() {
    // Start wait_for_new_work in background
    // Insert new workflow
    // Assert wait returns true (notified)
}
```

---

## Summary

| Task | Files | Complexity |

|------|-------|------------|

| WorkDistributor batch + notify | `work_distributor.rs` | High |

| Workflow payload validation | `postgres/mod.rs`, `postgres/workflow/state.rs`, `postgres/workflow/mod.rs` | Medium |

| Safe index migration | `migrations/20251210190500_partial_indexes.sql` | Low |

| Tests | `integration_tests.rs` | Medium |