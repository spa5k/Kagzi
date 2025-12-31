use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use kagzi::{Context, Retry};
use kagzi_proto::kagzi::WorkflowStatus as ProtoWorkflowStatus;
use serde::{Deserialize, Serialize};
use tests::common::{TestConfig, TestHarness, wait_for_status};
use tonic::Request;
use uuid::Uuid;

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
struct GreetingInput {
    name: String,
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
struct GreetingOutput {
    greeting: String,
}

#[tokio::test]
async fn happy_path_workflow_execution() -> anyhow::Result<()> {
    let harness = TestHarness::new().await;
    let queue = "e2e-workflow-basic";

    let mut worker = harness
        .worker_builder(queue)
        .workflows([(
            "hello_world",
            |mut ctx: Context, input: GreetingInput| async move {
                let step = ctx
                    .step("make_greeting")
                    .run(|| async move { Ok(format!("Hello, {}!", input.name)) })
                    .await?;
                Ok(GreetingOutput { greeting: step })
            },
        )])
        .build()
        .await?;
    let shutdown = worker.shutdown_token();
    let worker_handle = tokio::spawn(async move { worker.run().await });

    let client = harness.client().await;
    let run = client
        .start("hello_world")
        .namespace(queue)
        .input(GreetingInput {
            name: "Tester".into(),
        })
        .r#await()
        .await?;
    let run_id = run.id;
    let run_uuid = Uuid::parse_str(&run_id)?;

    let wf = wait_for_status(
        &harness.server_url,
        &run_id,
        ProtoWorkflowStatus::Completed,
        40,
    )
    .await?;
    let output: GreetingOutput =
        serde_json::from_slice(&wf.output.unwrap().data).expect("output should decode");
    assert_eq!(
        output,
        GreetingOutput {
            greeting: "Hello, Tester!".into()
        }
    );
    let db_status = harness.db_workflow_status(&run_uuid).await?;
    assert_eq!(db_status, "COMPLETED");

    shutdown.cancel();
    let _ = worker_handle.await;
    Ok(())
}

#[tokio::test]
async fn workflow_with_multiple_steps() -> anyhow::Result<()> {
    let harness = TestHarness::new().await;
    let queue = "e2e-workflow-multistep";

    let mut worker = harness
        .worker_builder(queue)
        .workflows([(
            "multi_step",
            |mut ctx: Context, input: GreetingInput| async move {
                let first = ctx
                    .step("upper")
                    .run(|| async move { Ok(input.name.to_uppercase()) })
                    .await?;
                let second = ctx
                    .step("suffix")
                    .run(|| async move { Ok(format!("{}-suffix", first)) })
                    .await?;
                Ok(GreetingOutput { greeting: second })
            },
        )])
        .build()
        .await?;
    let shutdown = worker.shutdown_token();
    let worker_handle = tokio::spawn(async move { worker.run().await });

    let client = harness.client().await;
    let run = client
        .start("multi_step")
        .namespace(queue)
        .input(GreetingInput {
            name: "chain".into(),
        })
        .r#await()
        .await?;
    let run_id = run.id;
    let run_uuid = Uuid::parse_str(&run_id)?;

    let wf = wait_for_status(
        &harness.server_url,
        &run_id,
        ProtoWorkflowStatus::Completed,
        40,
    )
    .await?;
    let output: GreetingOutput = serde_json::from_slice(&wf.output.unwrap().data)?;
    assert_eq!(
        output.greeting, "CHAIN-suffix",
        "steps should execute sequentially"
    );
    let db_status = harness.db_workflow_status(&run_uuid).await?;
    assert_eq!(db_status, "COMPLETED");

    shutdown.cancel();
    let _ = worker_handle.await;
    Ok(())
}

#[tokio::test]
async fn workflow_failure_propagates_error() -> anyhow::Result<()> {
    let harness = TestHarness::new().await;
    let queue = "e2e-workflow-failure";

    let mut worker = harness
        .worker_builder(queue)
        .workflows([(
            "failer",
            |_ctx: Context, _input: GreetingInput| async move {
                Err::<(), anyhow::Error>(anyhow::anyhow!("boom"))
            },
        )])
        .build()
        .await?;
    let shutdown = worker.shutdown_token();
    let worker_handle = tokio::spawn(async move { worker.run().await });

    let client = harness.client().await;
    let run = client
        .start("failer")
        .namespace(queue)
        .input(GreetingInput { name: "X".into() })
        .r#await()
        .await?;
    let run_id = run.id;
    let run_uuid = Uuid::parse_str(&run_id)?;

    let wf = wait_for_status(
        &harness.server_url,
        &run_id,
        ProtoWorkflowStatus::Failed,
        40,
    )
    .await?;
    assert_eq!(wf.status, ProtoWorkflowStatus::Failed as i32);
    let detail = wf.error.unwrap_or_default();
    let message = detail.message;
    assert!(
        message.contains("boom"),
        "error should propagate; got {}",
        message
    );
    let db_status = harness.db_workflow_status(&run_uuid).await?;
    assert_eq!(db_status, "FAILED");

    shutdown.cancel();
    let _ = worker_handle.await;
    Ok(())
}

#[tokio::test]
async fn workflow_panic_treated_as_failure() -> anyhow::Result<()> {
    let harness = TestHarness::new().await;
    let queue = "e2e-workflow-panic";

    let mut worker = harness
        .worker_builder(queue)
        .workflows([(
            "panic_workflow",
            |mut ctx: Context, _input: GreetingInput| async move {
                ctx.step("panic_step")
                    .run(|| async move {
                        Err::<(), anyhow::Error>(anyhow::anyhow!(
                            "panic: simulated panic in workflow"
                        ))
                    })
                    .await?;

                Ok::<_, anyhow::Error>(GreetingOutput {
                    greeting: "unreachable".into(),
                })
            },
        )])
        .build()
        .await?;
    let shutdown = worker.shutdown_token();
    let worker_handle = tokio::spawn(async move { worker.run().await });

    let client = harness.client().await;
    let run = client
        .start("panic_workflow")
        .namespace(queue)
        .input(GreetingInput {
            name: "panic".into(),
        })
        .r#await()
        .await?;
    let run_id = run.id;
    let run_uuid = Uuid::parse_str(&run_id)?;

    harness
        .wait_for_db_status(&run_uuid, "FAILED", 80, Duration::from_millis(250))
        .await?;

    let wf = wait_for_status(
        &harness.server_url,
        &run_id,
        ProtoWorkflowStatus::Failed,
        160,
    )
    .await?;
    assert_eq!(wf.status, ProtoWorkflowStatus::Failed as i32);
    let error = wf.error.unwrap_or_default();
    let message = error.message;
    assert!(
        message.contains("panic"),
        "panic message should be recorded, got {}",
        message
    );

    shutdown.cancel();
    let _ = worker_handle.await;
    Ok(())
}

#[tokio::test]
async fn workflow_retries_until_success() -> anyhow::Result<()> {
    let harness = TestHarness::new().await;
    let queue = "e2e-workflow-retry";

    let attempts = Arc::new(AtomicUsize::new(0));

    let attempt_counter = attempts.clone();
    let mut worker = harness
        .worker_builder(queue)
        .retry(Retry::exponential(3).initial("200ms").max("400ms"))
        .workflows([(
            "flaky_then_ok",
            move |mut ctx: Context, _input: GreetingInput| {
                let attempt_counter = attempt_counter.clone();
                async move {
                    let attempt = attempt_counter.fetch_add(1, Ordering::SeqCst) + 1;
                    let final_attempt = ctx
                        .step("flaky_step")
                        .run(|| async move {
                            if attempt < 3 {
                                anyhow::bail!("attempt {attempt} failed");
                            }
                            Ok::<_, anyhow::Error>(attempt)
                        })
                        .await?;
                    Ok(GreetingOutput {
                        greeting: format!("done-{final_attempt}"),
                    })
                }
            },
        )])
        .build()
        .await?;
    let shutdown = worker.shutdown_token();
    let worker_handle = tokio::spawn(async move { worker.run().await });

    let client = harness.client().await;
    let run = client
        .start("flaky_then_ok")
        .namespace(queue)
        .input(GreetingInput {
            name: "retry".into(),
        })
        .r#await()
        .await?;
    let run_id = run.id;
    let run_uuid = Uuid::parse_str(&run_id)?;

    let wf = wait_for_status(
        &harness.server_url,
        &run_id,
        ProtoWorkflowStatus::Completed,
        50,
    )
    .await?;
    assert_eq!(wf.status, ProtoWorkflowStatus::Completed as i32);

    let output: GreetingOutput = serde_json::from_slice(&wf.output.unwrap().data)?;
    assert_eq!(output.greeting, "done-3", "should succeed on third attempt");

    let attempt_total = attempts.load(Ordering::SeqCst);
    assert_eq!(
        attempt_total, 3,
        "step should retry until third attempt, got {}",
        attempt_total
    );
    let step_attempts = harness
        .db_step_attempt_count(&run_uuid, "flaky_step")
        .await?;
    assert_eq!(
        step_attempts, 3,
        "db should record three attempts for flaky_step"
    );

    shutdown.cancel();
    let _ = worker_handle.await;
    Ok(())
}

#[tokio::test]
async fn cancel_running_workflow_interrupts_execution() -> anyhow::Result<()> {
    use kagzi_proto::kagzi::CancelWorkflowRequest;
    use kagzi_proto::kagzi::workflow_service_client::WorkflowServiceClient;

    let harness = TestHarness::with_config(TestConfig {
        poll_timeout_secs: 1,
        ..Default::default()
    })
    .await;
    let queue = "e2e-workflow-cancel";

    let started = Arc::new(AtomicUsize::new(0));

    let started_flag = started.clone();
    let mut worker = harness
        .worker_builder(queue)
        .workflows([(
            "cancellable",
            move |_ctx: Context, _input: GreetingInput| {
                let started_flag = started_flag.clone();
                async move {
                    started_flag.fetch_add(1, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    Ok::<_, anyhow::Error>(GreetingOutput {
                        greeting: "should-not-complete".into(),
                    })
                }
            },
        )])
        .build()
        .await?;
    let shutdown = worker.shutdown_token();
    let worker_handle = tokio::spawn(async move { worker.run().await });

    let client = harness.client().await;
    let run = client
        .start("cancellable")
        .namespace(queue)
        .input(GreetingInput {
            name: "cancel-me".into(),
        })
        .r#await()
        .await?;
    let run_id = run.id;
    let run_uuid = Uuid::parse_str(&run_id)?;

    let mut execution_wait_count = 0;
    while started.load(Ordering::SeqCst) == 0 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        execution_wait_count += 1;
        if execution_wait_count > 100 {
            anyhow::bail!("workflow did not start execution within timeout");
        }
    }
    let mut workflow_client = WorkflowServiceClient::connect(harness.server_url.clone()).await?;
    workflow_client
        .cancel_workflow(Request::new(CancelWorkflowRequest {
            run_id: run_id.clone(),
            namespace_id: "default".into(),
        }))
        .await?;

    let wf = wait_for_status(
        &harness.server_url,
        &run_id,
        ProtoWorkflowStatus::Cancelled,
        30,
    )
    .await?;
    assert_eq!(wf.status, ProtoWorkflowStatus::Cancelled as i32);

    let attempts = harness.db_workflow_attempts(&run_uuid).await?;
    assert_eq!(attempts, 1, "cancelled workflow should not be retried");
    let started_total = started.load(Ordering::SeqCst);
    assert_eq!(
        started_total, 1,
        "worker should start at most once even with cancel"
    );

    shutdown.cancel();
    let _ = worker_handle.await;
    Ok(())
}

#[tokio::test]
async fn cancel_pending_workflow_succeeds() -> anyhow::Result<()> {
    use kagzi_proto::kagzi::CancelWorkflowRequest;
    use kagzi_proto::kagzi::workflow_service_client::WorkflowServiceClient;

    let harness = TestHarness::with_config(TestConfig {
        poll_timeout_secs: 2,
        ..Default::default()
    })
    .await;
    let queue = "e2e-workflow-cancel-pending";

    let client = harness.client().await;
    let run = client
        .start("cancellable_pending")
        .namespace(queue)
        .input(GreetingInput {
            name: "pending".into(),
        })
        .r#await()
        .await?;
    let run_id = run.id;

    let mut workflow_client = WorkflowServiceClient::connect(harness.server_url.clone()).await?;
    workflow_client
        .cancel_workflow(Request::new(CancelWorkflowRequest {
            run_id: run_id.clone(),
            namespace_id: "default".into(),
        }))
        .await?;

    let wf = wait_for_status(
        &harness.server_url,
        &run_id,
        ProtoWorkflowStatus::Cancelled,
        20,
    )
    .await?;
    assert_eq!(wf.status, ProtoWorkflowStatus::Cancelled as i32);
    Ok(())
}

#[tokio::test]
async fn cancel_sleeping_workflow_succeeds() -> anyhow::Result<()> {
    use kagzi_proto::kagzi::CancelWorkflowRequest;
    use kagzi_proto::kagzi::workflow_service_client::WorkflowServiceClient;

    let harness = TestHarness::with_config(TestConfig {
        poll_timeout_secs: 1,
        ..Default::default()
    })
    .await;
    let queue = "e2e-workflow-cancel-sleeping";

    let mut worker = harness
        .worker_builder(queue)
        .workflows([(
            "sleep_then_cancel",
            |mut ctx: Context, _input: GreetingInput| async move {
                ctx.sleep("sleep", "5s").await?;
                Ok::<_, anyhow::Error>(GreetingOutput {
                    greeting: "never".into(),
                })
            },
        )])
        .build()
        .await?;
    let shutdown = worker.shutdown_token();
    let worker_handle = tokio::spawn(async move { worker.run().await });

    let client = harness.client().await;
    let run = client
        .start("sleep_then_cancel")
        .namespace(queue)
        .input(GreetingInput {
            name: "sleep".into(),
        })
        .r#await()
        .await?;
    let run_id = run.id;
    let run_uuid = Uuid::parse_str(&run_id)?;
    harness
        .wait_for_db_status(&run_uuid, "SLEEPING", 20, Duration::from_millis(100))
        .await?;

    let mut workflow_client = WorkflowServiceClient::connect(harness.server_url.clone()).await?;
    workflow_client
        .cancel_workflow(Request::new(CancelWorkflowRequest {
            run_id: run_id.clone(),
            namespace_id: "default".into(),
        }))
        .await?;

    let wf = wait_for_status(
        &harness.server_url,
        &run_id,
        ProtoWorkflowStatus::Cancelled,
        30,
    )
    .await?;
    assert_eq!(wf.status, ProtoWorkflowStatus::Cancelled as i32);

    shutdown.cancel();
    let _ = worker_handle.await;
    Ok(())
}

#[tokio::test]
async fn cancel_completed_workflow_fails() -> anyhow::Result<()> {
    use kagzi_proto::kagzi::CancelWorkflowRequest;
    use kagzi_proto::kagzi::workflow_service_client::WorkflowServiceClient;

    let harness = TestHarness::new().await;
    let queue = "e2e-workflow-cancel-completed";

    let mut worker = harness
        .worker_builder(queue)
        .workflows([(
            "already_done",
            |_ctx: Context, _input: GreetingInput| async move {
                Ok::<_, anyhow::Error>(GreetingOutput {
                    greeting: "done".into(),
                })
            },
        )])
        .build()
        .await?;
    let shutdown = worker.shutdown_token();
    let worker_handle = tokio::spawn(async move { worker.run().await });

    let client = harness.client().await;
    let run = client
        .start("already_done")
        .namespace(queue)
        .input(GreetingInput {
            name: "complete".into(),
        })
        .r#await()
        .await?;
    let run_id = run.id;

    let wf = wait_for_status(
        &harness.server_url,
        &run_id,
        ProtoWorkflowStatus::Completed,
        20,
    )
    .await?;
    assert_eq!(wf.status, ProtoWorkflowStatus::Completed as i32);

    let mut workflow_client = WorkflowServiceClient::connect(harness.server_url.clone()).await?;
    let cancel_result = workflow_client
        .cancel_workflow(Request::new(CancelWorkflowRequest {
            run_id: run_id.clone(),
            namespace_id: "default".into(),
        }))
        .await;
    assert!(
        cancel_result.is_err(),
        "cancel on completed workflow should fail"
    );

    let wf_after = wait_for_status(
        &harness.server_url,
        &run_id,
        ProtoWorkflowStatus::Completed,
        20,
    )
    .await?;
    assert_eq!(wf_after.status, ProtoWorkflowStatus::Completed as i32);

    shutdown.cancel();
    let _ = worker_handle.await;
    Ok(())
}

#[tokio::test]
async fn cancel_idempotent_on_same_workflow() -> anyhow::Result<()> {
    use kagzi_proto::kagzi::CancelWorkflowRequest;
    use kagzi_proto::kagzi::workflow_service_client::WorkflowServiceClient;

    let harness = TestHarness::with_config(TestConfig {
        poll_timeout_secs: 1,
        ..Default::default()
    })
    .await;
    let queue = "e2e-workflow-cancel-idempotent";

    let mut worker = harness
        .worker_builder(queue)
        .workflows([(
            "cancel_twice",
            |_ctx: Context, _input: GreetingInput| async move {
                tokio::time::sleep(Duration::from_secs(5)).await;
                Ok::<_, anyhow::Error>(GreetingOutput {
                    greeting: "never".into(),
                })
            },
        )])
        .build()
        .await?;
    let shutdown = worker.shutdown_token();
    let worker_handle = tokio::spawn(async move { worker.run().await });

    let client = harness.client().await;
    let run = client
        .start("cancel_twice")
        .namespace(queue)
        .input(GreetingInput {
            name: "cancel".into(),
        })
        .r#await()
        .await?;

    let mut workflow_client = WorkflowServiceClient::connect(harness.server_url.clone()).await?;
    for _ in 0..2 {
        let _ = workflow_client
            .cancel_workflow(Request::new(CancelWorkflowRequest {
                run_id: run.id.clone(),
                namespace_id: "default".into(),
            }))
            .await;
    }

    let wf = wait_for_status(
        &harness.server_url,
        &run.id,
        ProtoWorkflowStatus::Cancelled,
        20,
    )
    .await?;
    assert_eq!(wf.status, ProtoWorkflowStatus::Cancelled as i32);

    shutdown.cancel();
    let _ = worker_handle.await;
    Ok(())
}

#[tokio::test]
async fn workflow_retry_policy_applied_on_failure() -> anyhow::Result<()> {
    let harness = TestHarness::with_config(TestConfig {
        watchdog_interval_secs: 1,
        ..Default::default()
    })
    .await;
    let queue = "e2e-workflow-retry-policy";

    let attempts = Arc::new(AtomicUsize::new(0));
    let counter = attempts.clone();
    let mut worker = harness
        .worker_builder(queue)
        .workflows([(
            "policy_retry",
            move |mut ctx: Context, _input: GreetingInput| {
                let counter = counter.clone();
                async move {
                    let attempt = counter.fetch_add(1, Ordering::SeqCst) + 1;
                    ctx.step("policy_step")
                        .run(|| async move {
                            if attempt < 3 {
                                anyhow::bail!("attempt {attempt} failed");
                            }
                            Ok::<_, anyhow::Error>(())
                        })
                        .await?;
                    Ok(GreetingOutput {
                        greeting: format!("ok-{attempt}"),
                    })
                }
            },
        )])
        .build()
        .await?;
    let shutdown = worker.shutdown_token();
    let worker_handle = tokio::spawn(async move { worker.run().await });

    let client = harness.client().await;
    let run = client
        .start("policy_retry")
        .namespace(queue)
        .input(GreetingInput {
            name: "workflow-retry".into(),
        })
        .r#await()
        .await?;
    let run_id = run.id;
    let run_uuid = Uuid::parse_str(&run_id)?;

    let wf = wait_for_status(
        &harness.server_url,
        &run_id,
        ProtoWorkflowStatus::Completed,
        50,
    )
    .await?;
    assert_eq!(wf.status, ProtoWorkflowStatus::Completed as i32);

    assert_eq!(
        attempts.load(Ordering::SeqCst),
        3,
        "workflow-level retry policy should drive step retries"
    );
    let step_attempts = harness
        .db_step_attempt_count(&run_uuid, "policy_step")
        .await?;
    assert_eq!(step_attempts, 3, "step should be attempted three times");

    shutdown.cancel();
    let _ = worker_handle.await;
    Ok(())
}

#[tokio::test]
async fn workflow_retry_exhausted_marks_failed() -> anyhow::Result<()> {
    let harness = TestHarness::with_config(TestConfig {
        watchdog_interval_secs: 1,
        ..Default::default()
    })
    .await;
    let queue = "e2e-workflow-retry-exhausted";

    let mut worker = harness
        .worker_builder(queue)
        .workflows([(
            "retry_exhaust",
            |_ctx: Context, _input: GreetingInput| async move {
                Err::<GreetingOutput, _>(anyhow::anyhow!("permanent failure"))
            },
        )])
        .build()
        .await?;
    let shutdown = worker.shutdown_token();
    let worker_handle = tokio::spawn(async move { worker.run().await });

    let client = harness.client().await;
    let run = client
        .start("retry_exhaust")
        .namespace(queue)
        .input(GreetingInput { name: "x".into() })
        .r#await()
        .await?;
    let run_id = run.id;
    let run_uuid = Uuid::parse_str(&run_id)?;

    let wf = wait_for_status(
        &harness.server_url,
        &run_id,
        ProtoWorkflowStatus::Failed,
        40,
    )
    .await?;
    assert_eq!(wf.status, ProtoWorkflowStatus::Failed as i32);

    let step_attempts = harness
        .db_step_attempt_count(&run_uuid, "retry_exhaust")
        .await?;
    assert!(
        step_attempts <= 2,
        "retries should stop after exhausting maximum attempts"
    );

    shutdown.cancel();
    let _ = worker_handle.await;
    Ok(())
}

#[tokio::test]
async fn workflow_retry_delay_respected() -> anyhow::Result<()> {
    let harness = TestHarness::with_config(TestConfig {
        watchdog_interval_secs: 1,
        ..Default::default()
    })
    .await;
    let queue = "e2e-workflow-retry-delay";

    let attempts = Arc::new(AtomicUsize::new(0));
    let counter = attempts.clone();
    let mut worker = harness
        .worker_builder(queue)
        .workflows([(
            "retry_delay",
            move |mut ctx: Context, _input: GreetingInput| {
                let counter = counter.clone();
                async move {
                    let attempt = counter.fetch_add(1, Ordering::SeqCst) + 1;
                    ctx.step("delay_step")
                        .run(|| async move {
                            if attempt == 1 {
                                anyhow::bail!("first attempt fails");
                            }
                            Ok::<_, anyhow::Error>(())
                        })
                        .await?;
                    Ok(GreetingOutput {
                        greeting: format!("attempt-{attempt}"),
                    })
                }
            },
        )])
        .build()
        .await?;
    let shutdown = worker.shutdown_token();
    let worker_handle = tokio::spawn(async move { worker.run().await });

    let client = harness.client().await;
    let start = std::time::Instant::now();
    let run = client
        .start("retry_delay")
        .namespace(queue)
        .input(GreetingInput {
            name: "delay".into(),
        })
        .r#await()
        .await?;
    let run_id = run.id;

    let wf = wait_for_status(
        &harness.server_url,
        &run_id,
        ProtoWorkflowStatus::Completed,
        40,
    )
    .await?;
    assert_eq!(wf.status, ProtoWorkflowStatus::Completed as i32);

    let elapsed = start.elapsed();
    assert!(
        elapsed >= Duration::from_millis(300),
        "retry should wait before next attempt (elapsed {:?})",
        elapsed
    );
    assert_eq!(
        attempts.load(Ordering::SeqCst),
        2,
        "exactly two attempts expected"
    );

    shutdown.cancel();
    let _ = worker_handle.await;
    Ok(())
}

#[tokio::test]
async fn step_retry_respects_backoff_policy() -> anyhow::Result<()> {
    let harness = TestHarness::with_config(TestConfig {
        watchdog_interval_secs: 1,
        ..Default::default()
    })
    .await;
    let queue = "e2e-step-retry-backoff";

    let attempts = Arc::new(AtomicUsize::new(0));
    let counter = attempts.clone();
    let mut worker = harness
        .worker_builder(queue)
        .retry(Retry::exponential(3).initial("200ms").max("400ms"))
        .workflows([(
            "step_backoff",
            move |mut ctx: Context, _input: GreetingInput| {
                let counter = counter.clone();
                async move {
                    let attempt = counter.fetch_add(1, Ordering::SeqCst) + 1;
                    ctx.step("backoff_step")
                        .run(|| async move {
                            if attempt < 3 {
                                anyhow::bail!("step attempt {attempt} failed");
                            }
                            Ok::<_, anyhow::Error>(attempt)
                        })
                        .await?;
                    Ok(GreetingOutput {
                        greeting: format!("done-{attempt}"),
                    })
                }
            },
        )])
        .build()
        .await?;
    let shutdown = worker.shutdown_token();
    let worker_handle = tokio::spawn(async move { worker.run().await });

    let client = harness.client().await;
    let start = std::time::Instant::now();
    let run = client
        .start("step_backoff")
        .namespace(queue)
        .input(GreetingInput {
            name: "step".into(),
        })
        .r#await()
        .await?;
    let run_id = run.id;
    let run_uuid = Uuid::parse_str(&run_id)?;

    let wf = wait_for_status(
        &harness.server_url,
        &run_id,
        ProtoWorkflowStatus::Completed,
        50,
    )
    .await?;
    assert_eq!(wf.status, ProtoWorkflowStatus::Completed as i32);

    let elapsed = start.elapsed();
    assert!(
        elapsed >= Duration::from_millis(300),
        "backoff should introduce visible delay (elapsed {:?})",
        elapsed
    );
    let step_attempts = harness
        .db_step_attempt_count(&run_uuid, "backoff_step")
        .await?;
    assert_eq!(step_attempts, 3, "step should retry until third attempt");

    shutdown.cancel();
    let _ = worker_handle.await;
    Ok(())
}

#[tokio::test]
async fn step_retry_non_retryable_error_stops_immediately() -> anyhow::Result<()> {
    let harness = TestHarness::with_config(TestConfig {
        watchdog_interval_secs: 1,
        ..Default::default()
    })
    .await;
    let queue = "e2e-step-retry-nonretryable";

    let mut worker = harness
        .worker_builder(queue)
        .retry(
            Retry::exponential(5)
                .initial("200ms")
                .max("400ms")
                .non_retryable(["fatal"]),
        )
        .workflows([(
            "non_retryable",
            |mut ctx: Context, _input: GreetingInput| async move {
                ctx.step("non_retryable_step")
                    .run(|| async move {
                        Err::<(), anyhow::Error>(anyhow::anyhow!("fatal: do not retry"))
                    })
                    .await?;
                Ok(GreetingOutput {
                    greeting: "should-not-complete".into(),
                })
            },
        )])
        .build()
        .await?;
    let shutdown = worker.shutdown_token();
    let worker_handle = tokio::spawn(async move { worker.run().await });

    let client = harness.client().await;
    let run = client
        .start("non_retryable")
        .namespace(queue)
        .input(GreetingInput {
            name: "fatal".into(),
        })
        .r#await()
        .await?;
    let run_id = run.id;
    let run_uuid = Uuid::parse_str(&run_id)?;

    let wf = wait_for_status(
        &harness.server_url,
        &run_id,
        ProtoWorkflowStatus::Failed,
        30,
    )
    .await?;
    assert_eq!(wf.status, ProtoWorkflowStatus::Failed as i32);

    let step_attempts = harness
        .db_step_attempt_count(&run_uuid, "non_retryable_step")
        .await?;
    assert_eq!(
        step_attempts, 1,
        "non-retryable error should stop after first attempt"
    );

    shutdown.cancel();
    let _ = worker_handle.await;
    Ok(())
}

#[tokio::test]
async fn step_retry_at_honored_before_reexecution() -> anyhow::Result<()> {
    let harness = TestHarness::with_config(TestConfig {
        watchdog_interval_secs: 1,
        ..Default::default()
    })
    .await;
    let queue = "e2e-step-retry-at";

    let attempts = Arc::new(AtomicUsize::new(0));
    let counter = attempts.clone();
    let mut worker = harness
        .worker_builder(queue)
        .retry(Retry::exponential(2).initial("800ms").max("800ms"))
        .workflows([(
            "retry_at",
            move |mut ctx: Context, _input: GreetingInput| {
                let counter = counter.clone();
                async move {
                    let attempt = counter.fetch_add(1, Ordering::SeqCst) + 1;
                    ctx.step("retry_at_step")
                        .run(|| async move {
                            if attempt == 1 {
                                anyhow::bail!("wait for retry_at");
                            }
                            Ok::<_, anyhow::Error>(attempt)
                        })
                        .await?;
                    Ok(GreetingOutput {
                        greeting: format!("attempt-{attempt}"),
                    })
                }
            },
        )])
        .build()
        .await?;
    let shutdown = worker.shutdown_token();
    let worker_handle = tokio::spawn(async move { worker.run().await });

    let client = harness.client().await;
    let start = std::time::Instant::now();
    let run = client
        .start("retry_at")
        .namespace(queue)
        .input(GreetingInput {
            name: "retry-at".into(),
        })
        .r#await()
        .await?;
    let run_id = run.id;
    let run_uuid = Uuid::parse_str(&run_id)?;

    let wf = wait_for_status(
        &harness.server_url,
        &run_id,
        ProtoWorkflowStatus::Completed,
        40,
    )
    .await?;
    assert_eq!(wf.status, ProtoWorkflowStatus::Completed as i32);

    let elapsed = start.elapsed();
    assert!(
        elapsed >= Duration::from_millis(400),
        "retry_at should delay re-execution (elapsed {:?})",
        elapsed
    );
    let step_attempts = harness
        .db_step_attempt_count(&run_uuid, "retry_at_step")
        .await?;
    assert_eq!(step_attempts, 2, "should retry exactly once");

    shutdown.cancel();
    let _ = worker_handle.await;
    Ok(())
}
