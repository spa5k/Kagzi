# Integration Test Plan

Comprehensive integration tests to cover edge cases and ensure system robustness.

---

## 1. Worker Lifecycle

### 1.1 Registration & Deregistration

- [ ] **worker_registration_returns_unique_id** - Each worker gets a unique ID on registration
- [ ] **worker_reregistration_same_queue_updates_existing** - Re-registering with same params updates rather than creates new
- [ ] **worker_deregistration_clears_from_pool** - Deregistered workers no longer receive work
- [ ] **worker_drain_mode_completes_active_before_stopping** - Draining worker finishes in-flight work but accepts no new tasks
- [ ] **worker_offline_after_missed_heartbeats** - Worker marked offline after missing N heartbeats

### 1.2 Heartbeat & Health

- [ ] **worker_heartbeat_extends_active_status** - Regular heartbeats keep worker active
- [ ] **worker_heartbeat_updates_active_count** - Heartbeat correctly reports active workflow count
- [ ] **stale_worker_detected_by_watchdog** - Watchdog marks workers as stale after threshold
- [ ] **stale_worker_workflows_rescheduled** - Workflows from stale workers are made claimable again

### 1.3 Multi-Worker Scenarios

- [ ] **multiple_workers_same_queue_fair_distribution** - Work is distributed fairly among workers
- [ ] **worker_with_different_workflow_types_only_receives_matching** - Type filtering works correctly
- [ ] **worker_concurrency_limit_respected** - Worker never exceeds max_concurrent setting
- [ ] **workflow_type_concurrency_limit_per_worker** - Per-type limits are enforced

---

## 2. Workflow Execution

### 2.1 Happy Path

- [ ] **simple_workflow_completes_successfully** - Basic workflow runs and completes
- [ ] **workflow_with_input_receives_correct_data** - Input is correctly deserialized
- [ ] **workflow_output_stored_and_retrievable** - Output is persisted and can be fetched
- [ ] **workflow_context_preserved_across_steps** - Context is maintained throughout execution

### 2.2 Failure Handling

- [ ] **workflow_failure_records_error_message** - Error details are captured
- [ ] **workflow_panic_treated_as_failure** - Panics are caught and recorded
- [ ] **workflow_timeout_triggers_failure** - Deadline exceeded causes failure
- [ ] **non_retryable_error_fails_immediately** - Non-retryable errors skip retry logic

### 2.3 Cancellation

- [ ] **cancel_pending_workflow_succeeds** - Can cancel before execution starts
- [ ] **cancel_running_workflow_interrupts_execution** - Running workflows can be stopped
- [ ] **cancel_sleeping_workflow_succeeds** - Sleeping workflows can be cancelled
- [ ] **cancel_completed_workflow_fails** - Cannot cancel already-completed workflows
- [ ] **cancel_idempotent_on_same_workflow** - Multiple cancel calls are safe

---

## 3. Step Execution & Replay

### 3.1 Step Lifecycle

- [ ] **step_begin_creates_pending_record** - BeginStep creates step in correct state
- [ ] **step_complete_stores_output** - CompleteStep persists result
- [ ] **step_fail_records_error_and_schedules_retry** - Failed steps are retried per policy
- [ ] **step_already_completed_returns_cached_output** - Replay returns cached data without re-execution

### 3.2 Deterministic Replay

- [ ] **replay_skips_completed_steps** - Completed steps return should_execute=false
- [ ] **replay_reexecutes_failed_steps** - Failed steps are retried on replay
- [ ] **replay_with_different_step_order_fails** - Non-deterministic execution is detected
- [ ] **step_id_collision_across_workflows_handled** - Same step ID in different workflows is fine

### 3.3 Step Retry

- [ ] **step_retry_respects_max_attempts** - Stops after max retries
- [ ] **step_retry_respects_backoff_policy** - Exponential backoff works correctly
- [ ] **step_retry_non_retryable_error_stops_immediately** - Non-retryable errors bypass retry
- [ ] **step_retry_at_honored_before_reexecution** - Cannot retry before scheduled time

---

## 4. Sleep & Timer Handling

### 4.1 Basic Sleep

- [ ] **sleep_transitions_workflow_to_sleeping** - Sleep RPC changes status correctly
- [ ] **sleep_wake_up_at_set_correctly** - wake_up_at is computed from duration
- [ ] **sleep_zero_duration_is_noop** - Zero-second sleep returns immediately
- [ ] **sleep_max_duration_accepted** - 30-day sleep is allowed
- [ ] **sleep_over_max_duration_rejected** - >30 days is rejected

### 4.2 Sleep Recovery

- [ ] **sleeping_workflow_claimable_after_wake_up** - Expired sleeps become claimable
- [ ] **watchdog_wakes_sleeping_workflows_in_batches** - Batch wake-up works
- [ ] **worker_can_directly_claim_expired_sleeping_workflow** - Direct claim from SLEEPING state
- [ ] **sleep_step_replay_skips_execution** - On replay, completed sleep returns immediately

### 4.3 Sleep Edge Cases

- [ ] **multiple_sleeps_in_sequence** - Workflow with multiple sleeps works correctly
- [ ] **sleep_with_worker_death_mid_sleep** - New worker picks up after original dies
- [ ] **sleep_during_worker_shutdown_completes_step** - Graceful shutdown finishes sleep step

---

## 5. Concurrency & Race Conditions

### 5.1 Claim Races

- [ ] **concurrent_poll_same_workflow_only_one_claims** - FOR UPDATE SKIP LOCKED works
- [ ] **rapid_poll_burst_no_duplicate_claims** - High poll rate doesn't cause issues
- [ ] **claim_during_status_transition_handled** - Race between claim and status change

### 5.2 WorkDistributor Races

- [ ] **poll_timeout_doesnt_orphan_claimed_workflow** - Timed-out polls don't leave orphans
- [ ] **work_distributor_channel_close_prevents_claim** - Closed channels skip claiming
- [ ] **concurrent_work_distributor_requests_handled** - Multiple pending requests processed correctly

### 5.3 Workflow State Races

- [ ] **complete_and_fail_race_one_wins** - Only one terminal state is recorded
- [ ] **cancel_during_completion_race** - Cancel vs complete race is handled
- [ ] **step_complete_during_workflow_cancel** - In-flight steps during cancel

---

## 6. Retry & Recovery

### 6.1 Workflow Retry

- [ ] **workflow_retry_policy_applied_on_failure** - Retries happen per policy
- [ ] **workflow_retry_increments_attempt_counter** - Attempt count increases
- [ ] **workflow_retry_exhausted_marks_failed** - Max retries reached = permanent failure
- [ ] **workflow_retry_delay_respected** - Backoff delay is honored

### 6.2 Orphan Recovery

- [ ] **orphaned_workflow_detected_by_watchdog** - Lock expiration triggers detection
- [ ] **orphaned_workflow_rescheduled_with_backoff** - Retry scheduled with delay
- [ ] **orphan_recovery_increments_attempt** - Attempt count reflects recovery
- [ ] **orphan_with_exhausted_retries_fails** - No infinite orphan loop

### 6.3 Network Failure Simulation

- [ ] **worker_reconnects_after_network_blip** - Temporary disconnection handled
- [ ] **in_flight_rpc_timeout_doesnt_corrupt_state** - Timeout mid-RPC is safe
- [ ] **server_restart_workers_reregister** - Workers reconnect after server restart

---

## 7. Scheduling (Cron)

### 7.1 Basic Scheduling

- [ ] **schedule_fires_at_next_cron_time** - Workflow created at scheduled time
- [ ] **schedule_advances_after_fire** - next_fire_at updated after execution
- [ ] **schedule_invalid_cron_rejected** - Bad cron expressions fail validation
- [ ] **schedule_pause_stops_firing** - Paused schedules don't create workflows

### 7.2 Catchup & Missed Fires

- [ ] **schedule_catchup_fires_missed_runs** - Missed runs are caught up
- [ ] **schedule_max_catchup_limits_burst** - Catchup respects max_catchup setting
- [ ] **schedule_idempotency_prevents_duplicates** - Same fire time doesn't create duplicates

### 7.3 Schedule Management

- [ ] **schedule_update_recomputes_next_fire** - Changing cron recalculates timing
- [ ] **schedule_delete_stops_all_firing** - Deleted schedules stop immediately
- [ ] **schedule_list_returns_all_for_namespace** - Listing works with filters

---

## 8. Payload Handling

### 8.1 Size Limits

- [ ] **workflow_input_at_limit_accepted** - Max size input works
- [ ] **workflow_input_over_limit_rejected** - Oversized input fails with clear error
- [ ] **step_output_over_limit_rejected** - Large step outputs are rejected
- [ ] **workflow_output_over_limit_rejected** - Large final outputs are rejected

### 8.2 Serialization

- [ ] **json_input_deserialized_correctly** - JSON payloads work
- [ ] **binary_payload_preserved** - Non-JSON bytes are passed through
- [ ] **empty_payload_handled** - Null/empty inputs work
- [ ] **unicode_in_payload_preserved** - UTF-8 content survives round-trip

---

## 9. Multi-Tenancy & Namespaces

### 9.1 Namespace Isolation

- [ ] **workflows_isolated_by_namespace** - Different namespaces don't see each other's work
- [ ] **workers_only_poll_own_namespace** - Workers respect namespace boundaries
- [ ] **schedules_scoped_to_namespace** - Schedules are namespace-specific

### 9.2 Queue Isolation

- [ ] **different_queues_isolated** - Task queues are separate
- [ ] **worker_only_receives_registered_queue** - Queue filtering works

---

## 10. Observability & Debugging

### 10.1 Listing & Querying

- [ ] **list_workflows_pagination_works** - Cursor-based pagination
- [ ] **list_workflows_filters_by_status** - Status filtering works
- [ ] **list_steps_for_workflow** - Can retrieve all steps for a run
- [ ] **get_workflow_returns_complete_state** - Full state is retrievable

### 10.2 Tracing

- [ ] **correlation_id_propagated_through_execution** - IDs flow through steps
- [ ] **trace_id_maintained_across_replay** - Tracing survives restart

---

## 11. Edge Cases & Stress

### 11.1 Boundary Conditions

- [ ] **workflow_with_zero_steps_completes** - Empty workflow works
- [ ] **workflow_with_100_steps_completes** - Many steps work
- [ ] **very_long_workflow_type_name_accepted** - Long names work
- [ ] **special_characters_in_external_id** - Special chars in IDs handled

### 11.2 Stress & Load

- [ ] **1000_workflows_created_rapidly** - Burst creation works
- [ ] **100_workers_polling_same_queue** - High worker count works
- [ ] **workflow_with_rapid_step_succession** - Fast step execution works

### 11.3 Database Consistency

- [ ] **workflow_counter_accurate_after_burst** - Queue counters stay accurate
- [ ] **counter_reconciliation_fixes_drift** - Watchdog fixes counter drift
- [ ] **no_orphaned_payloads_after_workflow_delete** - Cleanup is complete

---

## 12. Security & Validation

### 12.1 Input Validation

- [ ] **invalid_uuid_rejected_gracefully** - Bad UUIDs return proper errors
- [ ] **empty_workflow_type_rejected** - Required fields enforced
- [ ] **negative_duration_rejected** - Invalid durations caught

### 12.2 Authorization (if applicable)

- [ ] **worker_cannot_complete_other_workers_workflow** - Lock ownership respected
- [ ] **namespace_cross_access_prevented** - Cannot access other namespaces

---

## Running Tests

```bash
# Run all integration tests
cargo test -p tests --test '*' -- --test-threads=1

# Run specific test file
cargo test -p tests --test e2e_resilience

# Run with verbose output
cargo test -p tests --test e2e_workflow -- --nocapture

# Run single test
cargo test -p tests --test e2e_workflow happy_path
```

## Test Harness Configuration

Tests can customize server behavior via `TestConfig`:

```rust
TestHarness::with_config(TestConfig {
    scheduler_interval_secs: 1,
    watchdog_interval_secs: 1,
    worker_stale_threshold_secs: 2,
    poll_timeout_secs: 2,
    ..Default::default()
})
```

---

## Priority Order

1. **Critical** (P0): Orphan recovery, claim races, sleep recovery
2. **High** (P1): Retry logic, cancellation, multi-worker
3. **Medium** (P2): Scheduling, payload limits, pagination
4. **Low** (P3): Stress tests, edge cases, observability
