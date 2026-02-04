use serde::{Deserialize, Serialize};
use tests::common::TestHarness;
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize)]
struct EchoInput {
    value: i32,
}

#[derive(Debug, Serialize, Deserialize)]
struct EchoOutput {
    value: i32,
}

#[tokio::test]
async fn workflow_executes_via_subscribe_work_and_claim_task() -> anyhow::Result<()> {
    let harness = TestHarness::new().await;

    let mut worker = harness
        .worker_builder("default")
        .workflows([("echo", |_ctx, input: EchoInput| async move {
            Ok(EchoOutput { value: input.value })
        })])
        .build()
        .await?;

    let shutdown = worker.shutdown_token();
    let worker_handle = tokio::spawn(async move { worker.run().await });

    let client = harness.client().await;
    let run = client
        .start("echo")
        .input(&EchoInput { value: 42 })?
        .send()
        .await?;

    let run_id = Uuid::parse_str(&run.id)?;

    let wait_result: anyhow::Result<()> = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let status = harness.db_workflow_status(&run_id).await?;
            if status == "COMPLETED" {
                return Ok(());
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    })
    .await
    .map_err(|_| anyhow::anyhow!("Timed out waiting for workflow to complete"))?;

    shutdown.cancel();
    let _ = worker_handle.await;

    wait_result
}

