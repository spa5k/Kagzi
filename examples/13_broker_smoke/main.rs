use std::time::Duration;

use anyhow::Context as _;
use kagzi::{SignalBackend, Worker};
use kagzi_proto::kagzi::workflow_service_client::WorkflowServiceClient;
use kagzi_proto::kagzi::{GetWorkflowRequest, WorkflowStatus};
use serde::{Deserialize, Serialize};
use tonic::Request;

#[derive(Debug, Serialize, Deserialize)]
struct Input {
    value: i32,
}

#[derive(Debug, Serialize, Deserialize)]
struct Output {
    value: i32,
}

fn backend_from_args() -> anyhow::Result<String> {
    let backend = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "server".to_string());
    Ok(backend)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let server =
        std::env::var("KAGZI_SERVER_URL").unwrap_or_else(|_| "http://localhost:50051".into());
    let backend = backend_from_args()?;

    let namespace = "default";
    let workflow_type = "broker_smoke";

    println!("🔌 Broker smoke test");
    println!("  server   = {server}");
    println!("  backend  = {backend}");

    let signal_backend = match backend.as_str() {
        "server" | "postgres" => SignalBackend::Server,
        "nats" => SignalBackend::Nats {
            url: std::env::var("KAGZI_NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".into()),
            subject_prefix: std::env::var("KAGZI_NATS_SUBJECT_PREFIX")
                .unwrap_or_else(|_| "kagzi.work".into()),
            queue_group: std::env::var("KAGZI_NATS_QUEUE_GROUP")
                .unwrap_or_else(|_| "kagzi-workers".into()),
        },
        "kafka" => SignalBackend::Kafka {
            brokers: std::env::var("KAGZI_KAFKA_BROKERS")
                .unwrap_or_else(|_| "localhost:9094".into()),
            topic: std::env::var("KAGZI_KAFKA_TOPIC").unwrap_or_else(|_| "kagzi-work".into()),
            group_id: std::env::var("KAGZI_KAFKA_GROUP_ID")
                .unwrap_or_else(|_| "kagzi-workers-broker-smoke".into()),
        },
        other => anyhow::bail!("Unknown backend '{other}'. Use: server|nats|kafka"),
    };

    let mut worker = Worker::new(&server)
        .namespace(namespace)
        .signal_backend(signal_backend)
        .workflows([(workflow_type, |_ctx, input: Input| async move {
            Ok(Output {
                value: input.value + 1,
            })
        })])
        .build()
        .await?;

    let shutdown = worker.shutdown_token();
    let worker_handle = tokio::spawn(async move { worker.run().await });

    // Give the wakeup backend a moment to subscribe before publishing.
    tokio::time::sleep(Duration::from_millis(500)).await;

    let client = kagzi::Kagzi::connect(&server).await?;
    let run = client
        .start(workflow_type)
        .namespace(namespace)
        .input(&Input { value: 41 })?
        .send()
        .await?;

    println!("🚀 Started workflow run_id={}", run.id);

    let mut wf_client = WorkflowServiceClient::connect(server.clone())
        .await
        .context("connect workflow service")?;

    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let resp = wf_client
                .get_workflow(Request::new(GetWorkflowRequest {
                    namespace: namespace.to_string(),
                    run_id: run.id.clone(),
                }))
                .await?;

            let Some(wf) = resp.get_ref().workflow.as_ref() else {
                tokio::time::sleep(Duration::from_millis(50)).await;
                continue;
            };

            let status = WorkflowStatus::try_from(wf.status).unwrap_or(WorkflowStatus::Unspecified);
            if status == WorkflowStatus::Completed {
                return Ok::<_, tonic::Status>(());
            }

            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .context("timed out waiting for workflow completion")?
    .context("get_workflow loop failed")?;

    println!("✅ Workflow completed");

    shutdown.cancel();
    let _ = worker_handle.await;

    Ok(())
}
