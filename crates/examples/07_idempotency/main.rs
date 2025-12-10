use std::env;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use kagzi::WorkflowContext;
use serde::{Deserialize, Serialize};

#[path = "../common.rs"]
mod common;

#[derive(Debug, Serialize, Deserialize)]
struct PaymentInput {
    order_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct MemoInput {
    value: i32,
}

static EXPENSIVE_CALLS: AtomicUsize = AtomicUsize::new(0);

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    common::init_tracing()?;
    let args: Vec<String> = env::args().collect();
    let variant = args.get(1).map(|s| s.as_str()).unwrap_or("external");

    let server = env::var("KAGZI_SERVER_URL").unwrap_or_else(|_| "http://localhost:50051".into());
    let queue = env::var("KAGZI_TASK_QUEUE").unwrap_or_else(|_| "idempotency".into());

    match variant {
        "external" => external_id_demo(&server, &queue).await?,
        "memo" => memoization_demo(&server, &queue).await?,
        _ => {
            eprintln!("Usage: cargo run -p kagzi --example 07_idempotency -- [external|memo]");
            std::process::exit(1);
        }
    }

    Ok(())
}

async fn external_id_demo(server: &str, queue: &str) -> anyhow::Result<()> {
    let mut worker = common::build_worker(server, queue).await?;
    worker.register("charge_payment", charge_payment);

    let mut client = common::connect_client(server).await?;
    let external_id = "order-123";
    let run1 = client
        .workflow(
            "charge_payment",
            queue,
            PaymentInput {
                order_id: external_id.into(),
            },
        )
        .id(external_id)
        .await?;
    let run2 = client
        .workflow(
            "charge_payment",
            queue,
            PaymentInput {
                order_id: external_id.into(),
            },
        )
        .id(external_id)
        .await?;

    tracing::info!(%run1, %run2, "Both calls return the same run id due to idempotency");
    tokio::spawn(async move {
        if let Err(e) = worker.run().await {
            tracing::error!(error = %e, "Worker error");
        }
    });
    tokio::time::sleep(Duration::from_secs(6)).await;
    Ok(())
}

async fn memoization_demo(server: &str, queue: &str) -> anyhow::Result<()> {
    let mut worker = common::build_worker(server, queue).await?;
    worker.register("memoized_workflow", memo_workflow);

    let mut client = common::connect_client(server).await?;
    let run = client
        .workflow("memoized_workflow", queue, MemoInput { value: 5 })
        .await?;

    tracing::info!(%run, "Started memoization workflow; expensive step should run once even if called twice");
    tokio::spawn(async move {
        if let Err(e) = worker.run().await {
            tracing::error!(error = %e, "Worker error");
        }
    });
    tokio::time::sleep(Duration::from_secs(6)).await;
    Ok(())
}

async fn charge_payment(
    _ctx: WorkflowContext,
    input: PaymentInput,
) -> anyhow::Result<serde_json::Value> {
    tracing::info!(order = %input.order_id, "processing payment");
    tokio::time::sleep(Duration::from_secs(2)).await;
    Ok(serde_json::json!({"status": "charged", "order_id": input.order_id}))
}

async fn memo_workflow(
    mut ctx: WorkflowContext,
    input: MemoInput,
) -> anyhow::Result<serde_json::Value> {
    let first = ctx
        .run_with_input("expensive", &input, expensive_calculation(input.value))
        .await?;
    let second = ctx
        .run_with_input("expensive", &input, expensive_calculation(input.value))
        .await?;

    Ok(serde_json::json!({
        "first": first,
        "second": second,
        "calls_made": EXPENSIVE_CALLS.load(Ordering::SeqCst),
    }))
}

async fn expensive_calculation(value: i32) -> anyhow::Result<i32> {
    let count = EXPENSIVE_CALLS.fetch_add(1, Ordering::SeqCst) + 1;
    tracing::info!(call = count, "expensive calculation running");
    tokio::time::sleep(Duration::from_secs(1)).await;
    Ok(value * value)
}
