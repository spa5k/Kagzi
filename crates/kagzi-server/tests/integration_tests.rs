//! Integration tests for kagzi-server
//!
//! These tests use testcontainers to spin up a real PostgreSQL instance
//! and test the full gRPC service implementation.

mod common;

use common::{TestHarness, from_json_bytes, json_bytes, make_request};
use kagzi_proto::kagzi::{
    BeginStepRequest, CompleteStepRequest, CompleteWorkflowRequest, FailWorkflowRequest,
    GetWorkflowRunRequest, HealthCheckRequest, ListWorkflowRunsRequest, PollActivityRequest,
    RecordHeartbeatRequest, ScheduleSleepRequest, StartWorkflowRequest, WorkflowStatus,
    workflow_service_server::WorkflowService,
};
use std::time::Duration;

// ============================================================================
// Happy Path Tests
// ============================================================================

/// Test: Start workflow → Poll → Execute → Complete
/// This is the basic happy path for a workflow execution.
#[tokio::test]
async fn test_workflow_happy_path_start_poll_complete() {
    let harness = TestHarness::new().await;

    // 1. Start a workflow
    let input_data = serde_json::json!({"user_id": 123, "action": "welcome"});
    let start_req = StartWorkflowRequest {
        workflow_id: "user-123".to_string(),
        task_queue: "default".to_string(),
        workflow_type: "WelcomeEmail".to_string(),
        input: json_bytes(&input_data),
        namespace_id: "test-ns".to_string(),
        idempotency_key: "".to_string(),
        context: vec![],
        deadline_at: None,
        version: "1.0".to_string(),
    };

    let response = harness
        .service
        .start_workflow(make_request(start_req))
        .await
        .expect("Failed to start workflow");

    let run_id = response.into_inner().run_id;
    assert!(!run_id.is_empty(), "run_id should not be empty");

    // 2. Verify workflow is PENDING
    let get_req = GetWorkflowRunRequest {
        run_id: run_id.clone(),
        namespace_id: "test-ns".to_string(),
    };

    let workflow = harness
        .service
        .get_workflow_run(make_request(get_req))
        .await
        .expect("Failed to get workflow")
        .into_inner()
        .workflow_run
        .expect("Workflow should exist");

    assert_eq!(workflow.status, WorkflowStatus::Pending as i32);
    assert_eq!(workflow.workflow_type, "WelcomeEmail");

    // 3. Poll for activity (worker claims the workflow)
    let poll_req = PollActivityRequest {
        task_queue: "default".to_string(),
        worker_id: "worker-1".to_string(),
        namespace_id: "test-ns".to_string(),
    };

    let poll_response = harness
        .service
        .poll_activity(make_request(poll_req))
        .await
        .expect("Failed to poll activity");

    let activity = poll_response.into_inner();
    assert_eq!(activity.run_id, run_id);
    assert_eq!(activity.workflow_type, "WelcomeEmail");

    // Verify input was passed correctly
    let received_input: serde_json::Value = from_json_bytes(&activity.workflow_input);
    assert_eq!(received_input["user_id"], 123);

    // 4. Verify workflow is now RUNNING
    let get_req = GetWorkflowRunRequest {
        run_id: run_id.clone(),
        namespace_id: "test-ns".to_string(),
    };

    let workflow = harness
        .service
        .get_workflow_run(make_request(get_req))
        .await
        .expect("Failed to get workflow")
        .into_inner()
        .workflow_run
        .expect("Workflow should exist");

    assert_eq!(workflow.status, WorkflowStatus::Running as i32);
    assert_eq!(workflow.worker_id, "worker-1");

    // 5. Complete the workflow
    let output_data = serde_json::json!({"email_sent": true, "timestamp": "2024-01-01T00:00:00Z"});
    let complete_req = CompleteWorkflowRequest {
        run_id: run_id.clone(),
        output: json_bytes(&output_data),
    };

    harness
        .service
        .complete_workflow(make_request(complete_req))
        .await
        .expect("Failed to complete workflow");

    // 6. Verify workflow is COMPLETED
    let get_req = GetWorkflowRunRequest {
        run_id: run_id.clone(),
        namespace_id: "test-ns".to_string(),
    };

    let workflow = harness
        .service
        .get_workflow_run(make_request(get_req))
        .await
        .expect("Failed to get workflow")
        .into_inner()
        .workflow_run
        .expect("Workflow should exist");

    assert_eq!(workflow.status, WorkflowStatus::Completed as i32);
    assert!(workflow.finished_at.is_some());

    // Verify output was stored
    let stored_output: serde_json::Value = from_json_bytes(&workflow.output);
    assert_eq!(stored_output["email_sent"], true);
}

/// Test: Start workflow → Poll → Fail
#[tokio::test]
async fn test_workflow_failure_path() {
    let harness = TestHarness::new().await;

    // 1. Start a workflow
    let start_req = StartWorkflowRequest {
        workflow_id: "failing-workflow".to_string(),
        task_queue: "default".to_string(),
        workflow_type: "FailingTask".to_string(),
        input: json_bytes(&serde_json::json!({})),
        namespace_id: "test-ns".to_string(),
        ..Default::default()
    };

    let run_id = harness
        .service
        .start_workflow(make_request(start_req))
        .await
        .expect("Failed to start workflow")
        .into_inner()
        .run_id;

    // 2. Poll and claim
    let poll_req = PollActivityRequest {
        task_queue: "default".to_string(),
        worker_id: "worker-1".to_string(),
        namespace_id: "test-ns".to_string(),
    };

    harness
        .service
        .poll_activity(make_request(poll_req))
        .await
        .expect("Failed to poll activity");

    // 3. Fail the workflow
    let fail_req = FailWorkflowRequest {
        run_id: run_id.clone(),
        error: "Connection timeout after 30s".to_string(),
    };

    harness
        .service
        .fail_workflow(make_request(fail_req))
        .await
        .expect("Failed to fail workflow");

    // 4. Verify workflow is FAILED
    let get_req = GetWorkflowRunRequest {
        run_id: run_id.clone(),
        namespace_id: "test-ns".to_string(),
    };

    let workflow = harness
        .service
        .get_workflow_run(make_request(get_req))
        .await
        .expect("Failed to get workflow")
        .into_inner()
        .workflow_run
        .expect("Workflow should exist");

    assert_eq!(workflow.status, WorkflowStatus::Failed as i32);
    assert_eq!(workflow.error, "Connection timeout after 30s");
    assert!(workflow.finished_at.is_some());
}

// ============================================================================
// Step Memoization Tests
// ============================================================================

/// Test: Step memoization returns cached results
#[tokio::test]
async fn test_step_memoization_returns_cached_result() {
    let harness = TestHarness::new().await;

    // 1. Start and claim a workflow
    let start_req = StartWorkflowRequest {
        workflow_id: "memo-test".to_string(),
        task_queue: "default".to_string(),
        workflow_type: "MemoTest".to_string(),
        input: json_bytes(&serde_json::json!({})),
        namespace_id: "test-ns".to_string(),
        ..Default::default()
    };

    let run_id = harness
        .service
        .start_workflow(make_request(start_req))
        .await
        .unwrap()
        .into_inner()
        .run_id;

    // Poll to mark as RUNNING
    let poll_req = PollActivityRequest {
        task_queue: "default".to_string(),
        worker_id: "worker-1".to_string(),
        namespace_id: "test-ns".to_string(),
    };
    harness
        .service
        .poll_activity(make_request(poll_req))
        .await
        .unwrap();

    // 2. Begin step (first time - should execute)
    let begin_req = BeginStepRequest {
        run_id: run_id.clone(),
        step_id: "fetch_user_data".to_string(),
        input: vec![],
    };

    let begin_response = harness
        .service
        .begin_step(make_request(begin_req))
        .await
        .unwrap()
        .into_inner();

    assert!(begin_response.should_execute, "First call should execute");
    assert!(
        begin_response.cached_result.is_empty(),
        "No cached result yet"
    );

    // 3. Complete the step with a result
    let step_output = serde_json::json!({"user": {"id": 1, "name": "Alice"}});
    let complete_step_req = CompleteStepRequest {
        run_id: run_id.clone(),
        step_id: "fetch_user_data".to_string(),
        output: json_bytes(&step_output),
    };

    harness
        .service
        .complete_step(make_request(complete_step_req))
        .await
        .unwrap();

    // 4. Begin step again (second time - should return cached)
    let begin_req = BeginStepRequest {
        run_id: run_id.clone(),
        step_id: "fetch_user_data".to_string(),
        input: vec![],
    };

    let begin_response = harness
        .service
        .begin_step(make_request(begin_req))
        .await
        .unwrap()
        .into_inner();

    assert!(
        !begin_response.should_execute,
        "Second call should NOT execute - use cache"
    );
    assert!(
        !begin_response.cached_result.is_empty(),
        "Should have cached result"
    );

    // Verify cached result matches original
    let cached: serde_json::Value = from_json_bytes(&begin_response.cached_result);
    assert_eq!(cached["user"]["name"], "Alice");
}

/// Test: Different steps are tracked independently
#[tokio::test]
async fn test_step_memoization_independent_steps() {
    let harness = TestHarness::new().await;

    // Setup workflow
    let start_req = StartWorkflowRequest {
        workflow_id: "multi-step".to_string(),
        task_queue: "default".to_string(),
        workflow_type: "MultiStep".to_string(),
        input: json_bytes(&serde_json::json!({})),
        namespace_id: "test-ns".to_string(),
        ..Default::default()
    };

    let run_id = harness
        .service
        .start_workflow(make_request(start_req))
        .await
        .unwrap()
        .into_inner()
        .run_id;

    // Poll to start
    harness
        .service
        .poll_activity(make_request(PollActivityRequest {
            task_queue: "default".to_string(),
            worker_id: "worker-1".to_string(),
            namespace_id: "test-ns".to_string(),
        }))
        .await
        .unwrap();

    // Complete step_a
    harness
        .service
        .complete_step(make_request(CompleteStepRequest {
            run_id: run_id.clone(),
            step_id: "step_a".to_string(),
            output: json_bytes(&serde_json::json!({"a": 1})),
        }))
        .await
        .unwrap();

    // step_a should be cached
    let begin_a = harness
        .service
        .begin_step(make_request(BeginStepRequest {
            run_id: run_id.clone(),
            step_id: "step_a".to_string(),
            input: vec![],
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(!begin_a.should_execute, "step_a should be cached");

    // step_b should NOT be cached (never executed)
    let begin_b = harness
        .service
        .begin_step(make_request(BeginStepRequest {
            run_id: run_id.clone(),
            step_id: "step_b".to_string(),
            input: vec![],
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(begin_b.should_execute, "step_b should execute");
}

// ============================================================================
// Durable Sleep Tests
// ============================================================================

/// Test: ScheduleSleep puts workflow to sleep, reaper wakes it up
#[tokio::test]
async fn test_durable_sleep_and_wake() {
    let harness = TestHarness::new().await;

    // 1. Start and claim workflow
    let start_req = StartWorkflowRequest {
        workflow_id: "sleep-test".to_string(),
        task_queue: "default".to_string(),
        workflow_type: "SleepTest".to_string(),
        input: json_bytes(&serde_json::json!({})),
        namespace_id: "test-ns".to_string(),
        ..Default::default()
    };

    let run_id = harness
        .service
        .start_workflow(make_request(start_req))
        .await
        .unwrap()
        .into_inner()
        .run_id;

    harness
        .service
        .poll_activity(make_request(PollActivityRequest {
            task_queue: "default".to_string(),
            worker_id: "worker-1".to_string(),
            namespace_id: "test-ns".to_string(),
        }))
        .await
        .unwrap();

    // 2. Schedule a 1-second sleep
    let sleep_req = ScheduleSleepRequest {
        run_id: run_id.clone(),
        duration_seconds: 1,
    };

    harness
        .service
        .schedule_sleep(make_request(sleep_req))
        .await
        .unwrap();

    // 3. Verify workflow is SLEEPING
    let workflow = harness
        .service
        .get_workflow_run(make_request(GetWorkflowRunRequest {
            run_id: run_id.clone(),
            namespace_id: "test-ns".to_string(),
        }))
        .await
        .unwrap()
        .into_inner()
        .workflow_run
        .unwrap();

    assert_eq!(workflow.status, WorkflowStatus::Sleeping as i32);
    assert!(workflow.wake_up_at.is_some());

    // 4. Wait for sleep to expire
    tokio::time::sleep(Duration::from_secs(2)).await;

    // 5. Run reaper manually to wake up sleeping workflows
    // (In production this runs in background, here we simulate one iteration)
    sqlx::query!(
        r#"
        UPDATE kagzi.workflow_runs
        SET status = 'PENDING',
            wake_up_at = NULL
        WHERE status = 'SLEEPING'
          AND wake_up_at <= NOW()
        "#
    )
    .execute(&harness.pool)
    .await
    .unwrap();

    // 6. Verify workflow is back to PENDING
    let workflow = harness
        .service
        .get_workflow_run(make_request(GetWorkflowRunRequest {
            run_id: run_id.clone(),
            namespace_id: "test-ns".to_string(),
        }))
        .await
        .unwrap()
        .into_inner()
        .workflow_run
        .unwrap();

    assert_eq!(
        workflow.status,
        WorkflowStatus::Pending as i32,
        "Workflow should wake up to PENDING"
    );
    assert!(
        workflow.wake_up_at.is_none(),
        "wake_up_at should be cleared"
    );

    // 7. Worker can claim it again
    let poll_response = harness
        .service
        .poll_activity(make_request(PollActivityRequest {
            task_queue: "default".to_string(),
            worker_id: "worker-2".to_string(),
            namespace_id: "test-ns".to_string(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(poll_response.run_id, run_id);
}

// ============================================================================
// Orphan Recovery Tests
// ============================================================================

/// Test: Orphaned workflows (worker crash) are recovered by reaper
#[tokio::test]
async fn test_orphan_recovery_expired_lock() {
    let harness = TestHarness::new().await;

    // 1. Start workflow
    let start_req = StartWorkflowRequest {
        workflow_id: "orphan-test".to_string(),
        task_queue: "default".to_string(),
        workflow_type: "OrphanTest".to_string(),
        input: json_bytes(&serde_json::json!({})),
        namespace_id: "test-ns".to_string(),
        ..Default::default()
    };

    let run_id = harness
        .service
        .start_workflow(make_request(start_req))
        .await
        .unwrap()
        .into_inner()
        .run_id;

    // 2. Poll to claim (sets lock)
    harness
        .service
        .poll_activity(make_request(PollActivityRequest {
            task_queue: "default".to_string(),
            worker_id: "crashed-worker".to_string(),
            namespace_id: "test-ns".to_string(),
        }))
        .await
        .unwrap();

    // 3. Simulate worker crash by expiring lock immediately
    let run_uuid = uuid::Uuid::parse_str(&run_id).unwrap();
    sqlx::query!(
        r#"
        UPDATE kagzi.workflow_runs
        SET locked_until = NOW() - INTERVAL '1 minute'
        WHERE run_id = $1
        "#,
        run_uuid
    )
    .execute(&harness.pool)
    .await
    .unwrap();

    // 4. Run reaper to recover orphan
    let orphan_result = sqlx::query!(
        r#"
        UPDATE kagzi.workflow_runs
        SET status = 'PENDING',
            locked_by = NULL,
            locked_until = NULL,
            attempts = attempts + 1
        WHERE status = 'RUNNING'
          AND locked_until IS NOT NULL
          AND locked_until < NOW()
        RETURNING run_id
        "#
    )
    .fetch_all(&harness.pool)
    .await
    .unwrap();

    assert_eq!(orphan_result.len(), 1, "Should recover 1 orphan");
    assert_eq!(orphan_result[0].run_id.to_string(), run_id);

    // 5. Verify workflow is back to PENDING with incremented attempts
    let workflow = harness
        .service
        .get_workflow_run(make_request(GetWorkflowRunRequest {
            run_id: run_id.clone(),
            namespace_id: "test-ns".to_string(),
        }))
        .await
        .unwrap()
        .into_inner()
        .workflow_run
        .unwrap();

    assert_eq!(workflow.status, WorkflowStatus::Pending as i32);
    assert_eq!(
        workflow.attempts, 2,
        "Attempts should be incremented (1 from initial pickup + 1 from orphan recovery)"
    );
    assert!(workflow.worker_id.is_empty(), "worker_id should be cleared");

    // 6. New worker can claim it
    let poll_response = harness
        .service
        .poll_activity(make_request(PollActivityRequest {
            task_queue: "default".to_string(),
            worker_id: "new-worker".to_string(),
            namespace_id: "test-ns".to_string(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(poll_response.run_id, run_id);
}

/// Test: Heartbeats extend lock and prevent orphan recovery
#[tokio::test]
async fn test_heartbeat_extends_lock() {
    let harness = TestHarness::new().await;

    // Setup workflow
    let start_req = StartWorkflowRequest {
        workflow_id: "heartbeat-test".to_string(),
        task_queue: "default".to_string(),
        workflow_type: "HeartbeatTest".to_string(),
        input: json_bytes(&serde_json::json!({})),
        namespace_id: "test-ns".to_string(),
        ..Default::default()
    };

    let run_id = harness
        .service
        .start_workflow(make_request(start_req))
        .await
        .unwrap()
        .into_inner()
        .run_id;

    harness
        .service
        .poll_activity(make_request(PollActivityRequest {
            task_queue: "default".to_string(),
            worker_id: "worker-1".to_string(),
            namespace_id: "test-ns".to_string(),
        }))
        .await
        .unwrap();

    // Record heartbeat
    let heartbeat_req = RecordHeartbeatRequest {
        run_id: run_id.clone(),
        worker_id: "worker-1".to_string(),
    };

    harness
        .service
        .record_heartbeat(make_request(heartbeat_req))
        .await
        .expect("Heartbeat should succeed");

    // Verify lock was extended (workflow still RUNNING)
    let workflow = harness
        .service
        .get_workflow_run(make_request(GetWorkflowRunRequest {
            run_id: run_id.clone(),
            namespace_id: "test-ns".to_string(),
        }))
        .await
        .unwrap()
        .into_inner()
        .workflow_run
        .unwrap();

    assert_eq!(workflow.status, WorkflowStatus::Running as i32);
    assert_eq!(workflow.worker_id, "worker-1");
}

/// Test: Wrong worker cannot heartbeat
#[tokio::test]
async fn test_heartbeat_wrong_worker_rejected() {
    let harness = TestHarness::new().await;

    // Setup workflow
    let start_req = StartWorkflowRequest {
        workflow_id: "wrong-worker".to_string(),
        task_queue: "default".to_string(),
        workflow_type: "Test".to_string(),
        input: json_bytes(&serde_json::json!({})),
        namespace_id: "test-ns".to_string(),
        ..Default::default()
    };

    let run_id = harness
        .service
        .start_workflow(make_request(start_req))
        .await
        .unwrap()
        .into_inner()
        .run_id;

    harness
        .service
        .poll_activity(make_request(PollActivityRequest {
            task_queue: "default".to_string(),
            worker_id: "worker-1".to_string(),
            namespace_id: "test-ns".to_string(),
        }))
        .await
        .unwrap();

    // Different worker tries to heartbeat
    let heartbeat_req = RecordHeartbeatRequest {
        run_id: run_id.clone(),
        worker_id: "worker-2".to_string(), // Wrong worker!
    };

    let result = harness
        .service
        .record_heartbeat(make_request(heartbeat_req))
        .await;

    assert!(
        result.is_err(),
        "Heartbeat from wrong worker should be rejected"
    );

    let status = result.unwrap_err();
    assert_eq!(
        status.code(),
        tonic::Code::FailedPrecondition,
        "Should return FAILED_PRECONDITION"
    );
}

// ============================================================================
// Idempotency Tests
// ============================================================================

/// Test: Same idempotency key returns existing workflow
#[tokio::test]
async fn test_idempotency_returns_existing_workflow() {
    let harness = TestHarness::new().await;

    let idempotency_key = "unique-key-12345";

    // First request
    let start_req = StartWorkflowRequest {
        workflow_id: "idem-test".to_string(),
        task_queue: "default".to_string(),
        workflow_type: "IdempotencyTest".to_string(),
        input: json_bytes(&serde_json::json!({"attempt": 1})),
        namespace_id: "test-ns".to_string(),
        idempotency_key: idempotency_key.to_string(),
        ..Default::default()
    };

    let first_run_id = harness
        .service
        .start_workflow(make_request(start_req))
        .await
        .unwrap()
        .into_inner()
        .run_id;

    // Second request with same key
    let start_req = StartWorkflowRequest {
        workflow_id: "idem-test-2".to_string(), // Different workflow_id
        task_queue: "different-queue".to_string(), // Different queue
        workflow_type: "DifferentType".to_string(), // Different type
        input: json_bytes(&serde_json::json!({"attempt": 2})), // Different input
        namespace_id: "test-ns".to_string(),
        idempotency_key: idempotency_key.to_string(), // Same key!
        ..Default::default()
    };

    let second_run_id = harness
        .service
        .start_workflow(make_request(start_req))
        .await
        .unwrap()
        .into_inner()
        .run_id;

    // Both should return the same run_id
    assert_eq!(
        first_run_id, second_run_id,
        "Same idempotency key should return same run_id"
    );

    // Verify only one workflow exists
    let list_response = harness
        .service
        .list_workflow_runs(make_request(ListWorkflowRunsRequest {
            namespace_id: "test-ns".to_string(),
            page_size: 100,
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(
        list_response.workflow_runs.len(),
        1,
        "Should have exactly 1 workflow"
    );
}

/// Test: Different idempotency keys create separate workflows
#[tokio::test]
async fn test_idempotency_different_keys_create_separate() {
    let harness = TestHarness::new().await;

    // First workflow
    let run_id_1 = harness
        .service
        .start_workflow(make_request(StartWorkflowRequest {
            workflow_id: "wf-1".to_string(),
            task_queue: "default".to_string(),
            workflow_type: "Test".to_string(),
            input: json_bytes(&serde_json::json!({})),
            namespace_id: "test-ns".to_string(),
            idempotency_key: "key-1".to_string(),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner()
        .run_id;

    // Second workflow with different key
    let run_id_2 = harness
        .service
        .start_workflow(make_request(StartWorkflowRequest {
            workflow_id: "wf-2".to_string(),
            task_queue: "default".to_string(),
            workflow_type: "Test".to_string(),
            input: json_bytes(&serde_json::json!({})),
            namespace_id: "test-ns".to_string(),
            idempotency_key: "key-2".to_string(),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner()
        .run_id;

    assert_ne!(
        run_id_1, run_id_2,
        "Different keys should create different workflows"
    );
}

/// Test: Idempotency is scoped to namespace
#[tokio::test]
async fn test_idempotency_scoped_to_namespace() {
    let harness = TestHarness::new().await;

    let idempotency_key = "same-key";

    // Workflow in namespace A
    let run_id_a = harness
        .service
        .start_workflow(make_request(StartWorkflowRequest {
            workflow_id: "wf".to_string(),
            task_queue: "default".to_string(),
            workflow_type: "Test".to_string(),
            input: json_bytes(&serde_json::json!({})),
            namespace_id: "namespace-a".to_string(),
            idempotency_key: idempotency_key.to_string(),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner()
        .run_id;

    // Workflow in namespace B with same key
    let run_id_b = harness
        .service
        .start_workflow(make_request(StartWorkflowRequest {
            workflow_id: "wf".to_string(),
            task_queue: "default".to_string(),
            workflow_type: "Test".to_string(),
            input: json_bytes(&serde_json::json!({})),
            namespace_id: "namespace-b".to_string(),
            idempotency_key: idempotency_key.to_string(),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner()
        .run_id;

    assert_ne!(
        run_id_a, run_id_b,
        "Same key in different namespaces should create different workflows"
    );
}

// ============================================================================
// List & Pagination Tests
// ============================================================================

/// Test: List workflows with pagination
#[tokio::test]
async fn test_list_workflows_pagination() {
    let harness = TestHarness::new().await;

    // Create 5 workflows
    for i in 0..5 {
        harness
            .service
            .start_workflow(make_request(StartWorkflowRequest {
                workflow_id: format!("wf-{}", i),
                task_queue: "default".to_string(),
                workflow_type: "Test".to_string(),
                input: json_bytes(&serde_json::json!({})),
                namespace_id: "test-ns".to_string(),
                ..Default::default()
            }))
            .await
            .unwrap();

        // Small delay to ensure different created_at times
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    // List first page (2 items)
    let page1 = harness
        .service
        .list_workflow_runs(make_request(ListWorkflowRunsRequest {
            namespace_id: "test-ns".to_string(),
            page_size: 2,
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(page1.workflow_runs.len(), 2);
    assert!(page1.has_more);
    assert!(!page1.next_page_token.is_empty());

    // List second page
    let page2 = harness
        .service
        .list_workflow_runs(make_request(ListWorkflowRunsRequest {
            namespace_id: "test-ns".to_string(),
            page_size: 2,
            page_token: page1.next_page_token,
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(page2.workflow_runs.len(), 2);
    assert!(page2.has_more);

    // List third page (should have 1 remaining)
    let page3 = harness
        .service
        .list_workflow_runs(make_request(ListWorkflowRunsRequest {
            namespace_id: "test-ns".to_string(),
            page_size: 2,
            page_token: page2.next_page_token,
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(page3.workflow_runs.len(), 1);
    assert!(!page3.has_more);

    // Verify no duplicates across pages
    let all_ids: Vec<String> = page1
        .workflow_runs
        .iter()
        .chain(page2.workflow_runs.iter())
        .chain(page3.workflow_runs.iter())
        .map(|w| w.run_id.clone())
        .collect();

    let unique_ids: std::collections::HashSet<_> = all_ids.iter().collect();
    assert_eq!(all_ids.len(), unique_ids.len(), "No duplicate workflows");
    assert_eq!(all_ids.len(), 5, "All 5 workflows returned");
}

/// Test: List workflows with status filter
#[tokio::test]
async fn test_list_workflows_status_filter() {
    let harness = TestHarness::new().await;

    // Create workflows with different statuses
    let _ = harness
        .service
        .start_workflow(make_request(StartWorkflowRequest {
            workflow_id: "pending-wf".to_string(),
            task_queue: "default".to_string(),
            workflow_type: "Test".to_string(),
            input: json_bytes(&serde_json::json!({})),
            namespace_id: "test-ns".to_string(),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner()
        .run_id;

    let run_id_2 = harness
        .service
        .start_workflow(make_request(StartWorkflowRequest {
            workflow_id: "completed-wf".to_string(),
            task_queue: "default".to_string(),
            workflow_type: "Test".to_string(),
            input: json_bytes(&serde_json::json!({})),
            namespace_id: "test-ns".to_string(),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner()
        .run_id;

    // Complete the second workflow
    harness
        .service
        .poll_activity(make_request(PollActivityRequest {
            task_queue: "default".to_string(),
            worker_id: "worker-1".to_string(),
            namespace_id: "test-ns".to_string(),
        }))
        .await
        .unwrap();

    harness
        .service
        .complete_workflow(make_request(CompleteWorkflowRequest {
            run_id: run_id_2.clone(),
            output: json_bytes(&serde_json::json!({})),
        }))
        .await
        .unwrap();

    // Filter by COMPLETED
    let completed = harness
        .service
        .list_workflow_runs(make_request(ListWorkflowRunsRequest {
            namespace_id: "test-ns".to_string(),
            filter_status: "COMPLETED".to_string(),
            page_size: 10,
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(completed.workflow_runs.len(), 1);
    assert_eq!(
        completed.workflow_runs[0].status,
        WorkflowStatus::Completed as i32
    );
}

// ============================================================================
// Health Check Tests
// ============================================================================

/// Test: Health check returns SERVING when DB is healthy
#[tokio::test]
async fn test_health_check_serving() {
    let harness = TestHarness::new().await;

    let response = harness
        .service
        .health_check(make_request(HealthCheckRequest {
            service: String::new(),
        }))
        .await
        .expect("Health check should succeed")
        .into_inner();

    assert_eq!(
        response.status,
        kagzi_proto::kagzi::health_check_response::ServingStatus::Serving as i32
    );
    assert!(response.timestamp.is_some());
}

// ============================================================================
// Edge Case Tests
// ============================================================================

/// Test: Get non-existent workflow returns NOT_FOUND
#[tokio::test]
async fn test_get_nonexistent_workflow() {
    let harness = TestHarness::new().await;

    let result = harness
        .service
        .get_workflow_run(make_request(GetWorkflowRunRequest {
            run_id: "00000000-0000-0000-0000-000000000000".to_string(),
            namespace_id: "test-ns".to_string(),
        }))
        .await;

    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code(), tonic::Code::NotFound);
}

/// Test: Invalid UUID returns INVALID_ARGUMENT
#[tokio::test]
async fn test_invalid_uuid_rejected() {
    let harness = TestHarness::new().await;

    let result = harness
        .service
        .get_workflow_run(make_request(GetWorkflowRunRequest {
            run_id: "not-a-uuid".to_string(),
            namespace_id: "test-ns".to_string(),
        }))
        .await;

    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code(), tonic::Code::InvalidArgument);
}

/// Test: Empty worker_id rejected for heartbeat
#[tokio::test]
async fn test_empty_worker_id_rejected() {
    let harness = TestHarness::new().await;

    let result = harness
        .service
        .record_heartbeat(make_request(RecordHeartbeatRequest {
            run_id: "00000000-0000-0000-0000-000000000000".to_string(),
            worker_id: "".to_string(),
        }))
        .await;

    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code(), tonic::Code::InvalidArgument);
}

/// Test: Cancel workflow in terminal state fails
#[tokio::test]
async fn test_cancel_completed_workflow_fails() {
    let harness = TestHarness::new().await;

    // Create and complete a workflow
    let run_id = harness
        .service
        .start_workflow(make_request(StartWorkflowRequest {
            workflow_id: "cancel-test".to_string(),
            task_queue: "default".to_string(),
            workflow_type: "Test".to_string(),
            input: json_bytes(&serde_json::json!({})),
            namespace_id: "test-ns".to_string(),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner()
        .run_id;

    harness
        .service
        .poll_activity(make_request(PollActivityRequest {
            task_queue: "default".to_string(),
            worker_id: "worker-1".to_string(),
            namespace_id: "test-ns".to_string(),
        }))
        .await
        .unwrap();

    harness
        .service
        .complete_workflow(make_request(CompleteWorkflowRequest {
            run_id: run_id.clone(),
            output: json_bytes(&serde_json::json!({})),
        }))
        .await
        .unwrap();

    // Try to cancel completed workflow
    let result = harness
        .service
        .cancel_workflow_run(make_request(kagzi_proto::kagzi::CancelWorkflowRunRequest {
            run_id: run_id.clone(),
            namespace_id: "test-ns".to_string(),
        }))
        .await;

    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code(), tonic::Code::FailedPrecondition);
}

// ============================================================================
// Observability Field Tests
// ============================================================================

/// Test: Verify all observability fields are correctly populated for a single workflow run
/// This tests:
/// - Workflow: version defaults to "1", attempts=1 after single pickup, started_at set
/// - Steps: input recorded, output recorded, started_at/finished_at set, attempt_number=1
#[tokio::test]
async fn test_observability_fields_single_run() {
    let harness = TestHarness::new().await;

    // 1. Start a workflow WITHOUT specifying version (should default to "1")
    let input_data = serde_json::json!({"user_id": 456, "email": "test@example.com"});
    let start_req = StartWorkflowRequest {
        workflow_id: "obs-test-user".to_string(),
        task_queue: "default".to_string(),
        workflow_type: "ObservabilityTest".to_string(),
        input: json_bytes(&input_data),
        namespace_id: "test-ns".to_string(),
        idempotency_key: format!("obs-test-{}", uuid::Uuid::new_v4()),
        context: vec![],
        deadline_at: None,
        version: "".to_string(), // Empty - should default to "1"
    };

    let response = harness
        .service
        .start_workflow(make_request(start_req))
        .await
        .unwrap();

    let run_id = response.into_inner().run_id;

    // 2. Poll to pick up the workflow
    let poll_response = harness
        .service
        .poll_activity(make_request(PollActivityRequest {
            task_queue: "default".to_string(),
            worker_id: "test-worker-obs".to_string(),
            namespace_id: "test-ns".to_string(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(poll_response.run_id, run_id);

    // 3. Verify workflow has version="1" and attempts=1 after single pickup
    let workflow = harness
        .service
        .get_workflow_run(make_request(GetWorkflowRunRequest {
            run_id: run_id.clone(),
            namespace_id: "test-ns".to_string(),
        }))
        .await
        .unwrap()
        .into_inner()
        .workflow_run
        .unwrap();

    assert_eq!(workflow.version, "1", "Version should default to '1'");
    assert_eq!(
        workflow.attempts, 1,
        "Attempts should be 1 after single pickup"
    );
    assert!(
        workflow.started_at.is_some(),
        "started_at should be set after pickup"
    );

    // 4. Begin step with input
    let step_input = serde_json::json!({"action": "fetch", "user_id": 456});
    let begin_req = BeginStepRequest {
        run_id: run_id.clone(),
        step_id: "fetch_data".to_string(),
        input: json_bytes(&step_input),
    };

    let begin_response = harness
        .service
        .begin_step(make_request(begin_req))
        .await
        .unwrap()
        .into_inner();

    assert!(begin_response.should_execute, "Step should execute");

    // 5. Complete the step with output
    let step_output = serde_json::json!({"data": {"name": "Test User", "status": "active"}});
    let complete_req = CompleteStepRequest {
        run_id: run_id.clone(),
        step_id: "fetch_data".to_string(),
        output: json_bytes(&step_output),
    };

    harness
        .service
        .complete_step(make_request(complete_req))
        .await
        .unwrap();

    // 6. Query step_runs directly to verify fields
    let step_run = sqlx::query!(
        r#"
        SELECT step_id, status, input, output, started_at, finished_at, attempt_number
        FROM kagzi.step_runs
        WHERE run_id = $1 AND step_id = $2 AND is_latest = true
        "#,
        uuid::Uuid::parse_str(&run_id).unwrap(),
        "fetch_data"
    )
    .fetch_one(&harness.pool)
    .await
    .unwrap();

    assert_eq!(step_run.step_id, "fetch_data");
    assert_eq!(step_run.status, "COMPLETED");
    assert_eq!(step_run.attempt_number, 1, "attempt_number should be 1");
    assert!(step_run.started_at.is_some(), "started_at should be set");
    assert!(step_run.finished_at.is_some(), "finished_at should be set");

    // Verify input was recorded
    let recorded_input: serde_json::Value = step_run.input.unwrap();
    assert_eq!(
        recorded_input["action"], "fetch",
        "Input should be recorded"
    );
    assert_eq!(recorded_input["user_id"], 456, "Input user_id should match");

    // Verify output was recorded
    let recorded_output: serde_json::Value = step_run.output.unwrap();
    assert_eq!(
        recorded_output["data"]["name"], "Test User",
        "Output should be recorded"
    );

    // 7. Complete the workflow
    let workflow_output = serde_json::json!({"result": "success"});
    harness
        .service
        .complete_workflow(make_request(CompleteWorkflowRequest {
            run_id: run_id.clone(),
            output: json_bytes(&workflow_output),
        }))
        .await
        .unwrap();

    // 8. Final verification
    let final_workflow = harness
        .service
        .get_workflow_run(make_request(GetWorkflowRunRequest {
            run_id: run_id.clone(),
            namespace_id: "test-ns".to_string(),
        }))
        .await
        .unwrap()
        .into_inner()
        .workflow_run
        .unwrap();

    assert_eq!(final_workflow.status, WorkflowStatus::Completed as i32);
    assert!(
        final_workflow.finished_at.is_some(),
        "finished_at should be set"
    );
    assert_eq!(
        final_workflow.attempts, 1,
        "Attempts should still be 1 for completed workflow"
    );
}

/// Test: Workflow with sleep has attempts=2 (picked up twice: before and after sleep)
#[tokio::test]
async fn test_observability_workflow_with_sleep_attempts() {
    let harness = TestHarness::new().await;

    // 1. Start workflow
    let start_req = StartWorkflowRequest {
        workflow_id: "sleep-test-user".to_string(),
        task_queue: "default".to_string(),
        workflow_type: "SleepTest".to_string(),
        input: json_bytes(&serde_json::json!({})),
        namespace_id: "test-ns".to_string(),
        idempotency_key: format!("sleep-test-{}", uuid::Uuid::new_v4()),
        context: vec![],
        deadline_at: None,
        version: "2.0".to_string(), // Explicit version
    };

    let run_id = harness
        .service
        .start_workflow(make_request(start_req))
        .await
        .unwrap()
        .into_inner()
        .run_id;

    // 2. First poll - pickup before sleep (attempts becomes 1)
    harness
        .service
        .poll_activity(make_request(PollActivityRequest {
            task_queue: "default".to_string(),
            worker_id: "worker-1".to_string(),
            namespace_id: "test-ns".to_string(),
        }))
        .await
        .unwrap();

    // Verify attempts = 1
    let workflow = harness
        .service
        .get_workflow_run(make_request(GetWorkflowRunRequest {
            run_id: run_id.clone(),
            namespace_id: "test-ns".to_string(),
        }))
        .await
        .unwrap()
        .into_inner()
        .workflow_run
        .unwrap();

    assert_eq!(
        workflow.version, "2.0",
        "Explicit version should be preserved"
    );
    assert_eq!(workflow.attempts, 1, "First pickup: attempts should be 1");

    // 3. Schedule sleep with short duration
    let sleep_step_id = "__sleep_0";
    harness
        .service
        .begin_step(make_request(BeginStepRequest {
            run_id: run_id.clone(),
            step_id: sleep_step_id.to_string(),
            input: json_bytes(&serde_json::json!({"duration_seconds": 1})),
        }))
        .await
        .unwrap();

    harness
        .service
        .schedule_sleep(make_request(ScheduleSleepRequest {
            run_id: run_id.clone(),
            duration_seconds: 1,
        }))
        .await
        .unwrap();

    // Complete the sleep step
    harness
        .service
        .complete_step(make_request(CompleteStepRequest {
            run_id: run_id.clone(),
            step_id: sleep_step_id.to_string(),
            output: json_bytes(&serde_json::json!(null)),
        }))
        .await
        .unwrap();

    // Verify sleep step has input recorded
    let sleep_step = sqlx::query!(
        r#"
        SELECT input, started_at, finished_at
        FROM kagzi.step_runs
        WHERE run_id = $1 AND step_id = $2 AND is_latest = true
        "#,
        uuid::Uuid::parse_str(&run_id).unwrap(),
        sleep_step_id
    )
    .fetch_one(&harness.pool)
    .await
    .unwrap();

    let sleep_input: serde_json::Value = sleep_step.input.unwrap();
    assert_eq!(
        sleep_input["duration_seconds"], 1,
        "Sleep input should record duration"
    );
    assert!(
        sleep_step.started_at.is_some(),
        "Sleep step should have started_at"
    );
    assert!(
        sleep_step.finished_at.is_some(),
        "Sleep step should have finished_at"
    );

    // 4. Wait for sleep to expire
    tokio::time::sleep(Duration::from_secs(2)).await;

    // 5. Second poll - pickup after sleep wakeup (attempts becomes 2)
    harness
        .service
        .poll_activity(make_request(PollActivityRequest {
            task_queue: "default".to_string(),
            worker_id: "worker-1".to_string(),
            namespace_id: "test-ns".to_string(),
        }))
        .await
        .unwrap();

    // 6. Verify attempts = 2 after wakeup
    let workflow_after_wakeup = harness
        .service
        .get_workflow_run(make_request(GetWorkflowRunRequest {
            run_id: run_id.clone(),
            namespace_id: "test-ns".to_string(),
        }))
        .await
        .unwrap()
        .into_inner()
        .workflow_run
        .unwrap();

    assert_eq!(
        workflow_after_wakeup.attempts, 2,
        "After sleep wakeup: attempts should be 2 (picked up twice)"
    );
}
