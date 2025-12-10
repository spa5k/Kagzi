#[path = "../common/mod.rs"]
mod common;

use std::time::Duration;

use common::TestHarness;
use kagzi::WorkflowContext;
use kagzi_proto::kagzi::workflow_service_client::WorkflowServiceClient;
use kagzi_proto::kagzi::{GetWorkflowRequest, WorkflowStatus};
use serde::{Deserialize, Serialize};
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

async fn fetch_workflow(
    server_url: &str,
    run_id: &str,
) -> anyhow::Result<kagzi_proto::kagzi::Workflow> {
    let mut client = WorkflowServiceClient::connect(server_url.to_string()).await?;
    let resp = client
        .get_workflow(Request::new(GetWorkflowRequest {
            run_id: run_id.to_string(),
            namespace_id: "default".to_string(),
        }))
        .await?;
    resp.into_inner()
        .workflow
        .ok_or_else(|| anyhow::anyhow!("workflow not found"))
}

async fn wait_for_status(
    server_url: &str,
    run_id: &str,
    expected: WorkflowStatus,
) -> anyhow::Result<kagzi_proto::kagzi::Workflow> {
    let max_attempts = 20;
    for attempt in 0..max_attempts {
        let wf = fetch_workflow(server_url, run_id).await?;
        if wf.status == expected as i32 {
            return Ok(wf);
        }
        if attempt == max_attempts - 1 {
            anyhow::bail!(
                "workflow {} did not reach status {:?}, last status={:?}",
                run_id,
                expected,
                wf.status
            );
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    unreachable!()
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

    let wf = wait_for_status(&harness.server_url, &run_id, WorkflowStatus::Completed).await?;
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

    let wf = wait_for_status(&harness.server_url, &run_id, WorkflowStatus::Completed).await?;
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

    let wf = wait_for_status(&harness.server_url, &run_id, WorkflowStatus::Failed).await?;
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
