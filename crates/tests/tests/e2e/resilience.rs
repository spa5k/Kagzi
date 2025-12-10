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
struct SleepInput {
    seconds: u64,
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
    attempts: usize,
) -> anyhow::Result<kagzi_proto::kagzi::Workflow> {
    for attempt in 0..attempts {
        let wf = fetch_workflow(server_url, run_id).await?;
        if wf.status == expected as i32 {
            return Ok(wf);
        }
        if attempt == attempts - 1 {
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
async fn workflow_survives_worker_restart_during_sleep() -> anyhow::Result<()> {
    let harness = TestHarness::new().await;
    let queue = "e2e-resilience-sleep";

    let mut worker1 = harness.worker(queue).await;
    worker1.register(
        "sleep_wf",
        |mut ctx: WorkflowContext, input: SleepInput| async move {
            ctx.sleep(Duration::from_secs(input.seconds)).await?;
            Ok::<_, anyhow::Error>(())
        },
    );
    let shutdown1 = worker1.shutdown_token();
    let handle1 = tokio::spawn(async move { worker1.run().await });

    let mut client = harness.client().await;
    let run_id = client
        .workflow("sleep_wf", queue, SleepInput { seconds: 2 })
        .await?;
    let run_uuid = Uuid::parse_str(&run_id)?;

    tokio::time::sleep(Duration::from_millis(500)).await;
    shutdown1.cancel();
    let _ = handle1.await;

    // Allow wake + reschedule.
    tokio::time::sleep(Duration::from_secs(4)).await;
    let sleeping = harness.db_workflow_status(&run_uuid).await?;
    assert!(
        sleeping == "SLEEPING" || sleeping == "RUNNING",
        "expected sleeping or running after worker crash, got {}",
        sleeping
    );

    let mut worker2 = harness.worker(queue).await;
    worker2.register(
        "sleep_wf",
        |mut ctx: WorkflowContext, input: SleepInput| async move {
            ctx.sleep(Duration::from_secs(input.seconds)).await?;
            Ok::<_, anyhow::Error>(())
        },
    );
    let shutdown2 = worker2.shutdown_token();
    let handle2 = tokio::spawn(async move { worker2.run().await });

    let mut last = None;
    for _ in 0..80 {
        let wf = fetch_workflow(&harness.server_url, &run_id).await?;
        last = Some(wf.clone());
        if wf.status == WorkflowStatus::Completed as i32
            || wf.status == WorkflowStatus::Running as i32
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    let wf = last.expect("workflow fetched");
    assert!(
        wf.status == WorkflowStatus::Completed as i32
            || wf.status == WorkflowStatus::Running as i32,
        "expected workflow to be running or completed after recovery, got {}",
        wf.status
    );
    let final_status = harness.db_workflow_status(&run_uuid).await?;
    assert!(
        final_status == "COMPLETED" || final_status == "RUNNING",
        "expected final DB status running/completed, got {}",
        final_status
    );

    shutdown2.cancel();
    let _ = handle2.await;
    Ok(())
}

#[tokio::test]
async fn orphaned_workflow_recovered_after_worker_death() -> anyhow::Result<()> {
    let harness = TestHarness::with_config(common::TestConfig {
        worker_stale_threshold_secs: 2,
        watchdog_interval_secs: 1,
        ..Default::default()
    })
    .await;
    let queue = "e2e-resilience-orphan";

    let mut worker1 = harness.worker(queue).await;
    worker1.register(
        "long_run",
        |_ctx: WorkflowContext, _input: SleepInput| async move {
            tokio::time::sleep(Duration::from_secs(10)).await;
            Ok::<_, anyhow::Error>(())
        },
    );
    let shutdown1 = worker1.shutdown_token();
    let handle1 = tokio::spawn(async move { worker1.run().await });

    let mut client = harness.client().await;
    let run_id = client
        .workflow("long_run", queue, SleepInput { seconds: 0 })
        .await?;
    let run_uuid = Uuid::parse_str(&run_id)?;

    tokio::time::sleep(Duration::from_millis(500)).await;
    shutdown1.cancel();
    let _ = handle1.await;

    // Wait for watchdog to mark stale + schedule retry.
    tokio::time::sleep(Duration::from_secs(6)).await;

    let mut worker2 = harness.worker(queue).await;
    worker2.register(
        "long_run",
        |_ctx: WorkflowContext, _input: SleepInput| async move { Ok::<_, anyhow::Error>(()) },
    );
    let shutdown2 = worker2.shutdown_token();
    let handle2 = tokio::spawn(async move { worker2.run().await });

    let wf = wait_for_status(&harness.server_url, &run_id, WorkflowStatus::Completed, 30).await?;
    assert_eq!(wf.status, WorkflowStatus::Completed as i32);

    // Locked_by should be cleared after recovery.
    let locked_by = harness.db_workflow_locked_by(&run_uuid).await?;
    assert!(
        locked_by.is_none(),
        "lock should be cleared after recovery, got {:?}",
        locked_by
    );

    shutdown2.cancel();
    let _ = handle2.await;
    Ok(())
}
