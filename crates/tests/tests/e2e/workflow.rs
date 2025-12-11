use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use kagzi::{RetryPolicy, WorkflowContext};
use kagzi_proto::kagzi::WorkflowStatus;
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

    let mut worker = harness.worker(queue).await;
    worker.register(
        "hello_world",
        |mut ctx: WorkflowContext, input: GreetingInput| async move {
            let step = ctx
                .run("make_greeting", async move {
                    Ok(format!("Hello, {}!", input.name))
                })
                .await?;
            Ok(GreetingOutput { greeting: step })
        },
    );
    let shutdown = worker.shutdown_token();
    let worker_handle = tokio::spawn(async move { worker.run().await });

    let mut client = harness.client().await;
    let run_id = client
        .workflow(
            "hello_world",
            queue,
            GreetingInput {
                name: "Tester".into(),
            },
        )
        .await?;
    let run_uuid = Uuid::parse_str(&run_id)?;

    let wf = wait_for_status(&harness.server_url, &run_id, WorkflowStatus::Completed, 40).await?;
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

    let mut worker = harness.worker(queue).await;
    worker.register(
        "multi_step",
        |mut ctx: WorkflowContext, input: GreetingInput| async move {
            let first = ctx
                .run("upper", async move { Ok(input.name.to_uppercase()) })
                .await?;
            let second = ctx
                .run("suffix", async move { Ok(format!("{}-suffix", first)) })
                .await?;
            Ok(GreetingOutput { greeting: second })
        },
    );
    let shutdown = worker.shutdown_token();
    let worker_handle = tokio::spawn(async move { worker.run().await });

    let mut client = harness.client().await;
    let run_id = client
        .workflow(
            "multi_step",
            queue,
            GreetingInput {
                name: "chain".into(),
            },
        )
        .await?;
    let run_uuid = Uuid::parse_str(&run_id)?;

    let wf = wait_for_status(&harness.server_url, &run_id, WorkflowStatus::Completed, 40).await?;
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

    let mut worker = harness.worker(queue).await;
    worker.register(
        "failer",
        |_ctx: WorkflowContext, _input: GreetingInput| async move {
            Err::<(), anyhow::Error>(anyhow::anyhow!("boom"))
        },
    );
    let shutdown = worker.shutdown_token();
    let worker_handle = tokio::spawn(async move { worker.run().await });

    let mut client = harness.client().await;
    let run_id = client
        .workflow("failer", queue, GreetingInput { name: "X".into() })
        .await?;
    let run_uuid = Uuid::parse_str(&run_id)?;

    let wf = wait_for_status(&harness.server_url, &run_id, WorkflowStatus::Failed, 40).await?;
    assert_eq!(wf.status, WorkflowStatus::Failed as i32);
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
async fn workflow_retries_until_success() -> anyhow::Result<()> {
    let harness = TestHarness::new().await;
    let queue = "e2e-workflow-retry";

    let attempts = Arc::new(AtomicUsize::new(0));

    let mut worker = kagzi::Worker::builder(&harness.server_url, queue)
        .default_step_retry(RetryPolicy {
            maximum_attempts: Some(3),
            initial_interval: Some(Duration::from_millis(200)),
            backoff_coefficient: Some(1.0),
            maximum_interval: Some(Duration::from_millis(400)),
            non_retryable_errors: vec![],
        })
        .build()
        .await?;
    let attempt_counter = attempts.clone();
    worker.register(
        "flaky_then_ok",
        move |mut ctx: WorkflowContext, _input: GreetingInput| {
            let attempt_counter = attempt_counter.clone();
            async move {
                let attempt = attempt_counter.fetch_add(1, Ordering::SeqCst) + 1;
                let final_attempt = ctx
                    .run("flaky_step", async move {
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
    );
    let shutdown = worker.shutdown_token();
    let worker_handle = tokio::spawn(async move { worker.run().await });

    let mut client = harness.client().await;
    let run_id = client
        .workflow(
            "flaky_then_ok",
            queue,
            GreetingInput {
                name: "retry".into(),
            },
        )
        .await?;
    let run_uuid = Uuid::parse_str(&run_id)?;

    let wf = wait_for_status(&harness.server_url, &run_id, WorkflowStatus::Completed, 50).await?;
    assert_eq!(wf.status, WorkflowStatus::Completed as i32);

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

    let mut worker = harness.worker(queue).await;
    let started_flag = started.clone();
    worker.register(
        "cancellable",
        move |_ctx: WorkflowContext, _input: GreetingInput| {
            let started_flag = started_flag.clone();
            async move {
                started_flag.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_secs(5)).await;
                Ok::<_, anyhow::Error>(GreetingOutput {
                    greeting: "should-not-complete".into(),
                })
            }
        },
    );
    let shutdown = worker.shutdown_token();
    let worker_handle = tokio::spawn(async move { worker.run().await });

    let mut client = harness.client().await;
    let run_id = client
        .workflow(
            "cancellable",
            queue,
            GreetingInput {
                name: "cancel-me".into(),
            },
        )
        .await?;
    let run_uuid = Uuid::parse_str(&run_id)?;

    // Let the worker start, then issue cancel.
    tokio::time::sleep(Duration::from_millis(500)).await;
    let mut workflow_client = WorkflowServiceClient::connect(harness.server_url.clone()).await?;
    workflow_client
        .cancel_workflow(Request::new(CancelWorkflowRequest {
            run_id: run_id.clone(),
            namespace_id: "default".into(),
        }))
        .await?;

    let wf = wait_for_status(&harness.server_url, &run_id, WorkflowStatus::Cancelled, 30).await?;
    assert_eq!(wf.status, WorkflowStatus::Cancelled as i32);

    // The workflow should not be retried or completed after cancellation.
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
