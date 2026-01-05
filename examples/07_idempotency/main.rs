use std::env;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use kagzi::{Context, Kagzi, Worker};
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
    let args: Vec<String> = env::args().collect();
    let variant = args.get(1).map(|s| s.as_str()).unwrap_or("external");

    let server = env::var("KAGZI_SERVER_URL").unwrap_or_else(|_| "http://localhost:50051".into());
    let namespace = env::var("KAGZI_NAMESPACE").unwrap_or_else(|_| "idempotency".into());

    match variant {
        "external" => external_id_demo(&server, &namespace).await?,
        "memo" => memoization_demo(&server, &namespace).await?,
        _ => {
            eprintln!("Usage: cargo run -p kagzi --example 07_idempotency -- [external|memo]");
            std::process::exit(1);
        }
    }

    Ok(())
}

async fn external_id_demo(server: &str, namespace: &str) -> anyhow::Result<()> {
    println!("🔑 External ID Example - demonstrates idempotent workflow execution\n");

    let mut worker = Worker::new(server)
        .namespace(namespace)
        .workflows([("charge_payment", charge_payment)])
        .build()
        .await?;

    println!("👷 Worker started");

    let client = Kagzi::connect(server).await?;
    let external_id = "order-123";
    let input = PaymentInput {
        order_id: external_id.into(),
    };
    let run1 = client
        .start("charge_payment")
        .namespace(namespace)
        .idempotency_key(external_id)
        .input(&input)?
        .send()
        .await?;
    let run2 = client
        .start("charge_payment")
        .namespace(namespace)
        .idempotency_key(external_id)
        .input(&input)?
        .send()
        .await?;

    println!(
        "✅ Both calls return the same run id due to idempotency: run1={}, run2={}",
        run1.id, run2.id
    );
    tokio::spawn(async move {
        if let Err(e) = worker.run().await {
            eprintln!("❌ Worker error: {:?}", e);
        }
    });
    tokio::time::sleep(Duration::from_secs(6)).await;
    println!("✅ Example complete\n");
    Ok(())
}

async fn memoization_demo(server: &str, namespace: &str) -> anyhow::Result<()> {
    println!("🧠 Memoization Example - demonstrates step result caching\n");

    let mut worker = Worker::new(server)
        .namespace(namespace)
        .workflows([("memoized_workflow", memo_workflow)])
        .build()
        .await?;

    println!("👷 Worker started");

    let client = Kagzi::connect(server).await?;
    let input = MemoInput { value: 5 };
    let run = client
        .start("memoized_workflow")
        .namespace(namespace)
        .input(&input)?
        .send()
        .await?;

    println!(
        "🚀 Started memoization workflow; expensive step should run once even if called twice: {}",
        run.id
    );
    tokio::spawn(async move {
        if let Err(e) = worker.run().await {
            eprintln!("❌ Worker error: {:?}", e);
        }
    });
    tokio::time::sleep(Duration::from_secs(6)).await;
    println!("✅ Example complete\n");
    Ok(())
}

async fn charge_payment(_ctx: Context, input: PaymentInput) -> anyhow::Result<serde_json::Value> {
    println!("processing payment: order={}", input.order_id);
    tokio::time::sleep(Duration::from_secs(2)).await;
    Ok(serde_json::json!({"status": "charged", "order_id": input.order_id}))
}

async fn memo_workflow(mut ctx: Context, input: MemoInput) -> anyhow::Result<serde_json::Value> {
    let first = ctx
        .step("expensive")
        .run(|| expensive_calculation(input.value))
        .await?;
    let second = ctx
        .step("expensive")
        .run(|| expensive_calculation(input.value))
        .await?;

    Ok(serde_json::json!({
        "first": first,
        "second": second,
        "calls_made": EXPENSIVE_CALLS.load(Ordering::SeqCst),
    }))
}

async fn expensive_calculation(value: i32) -> anyhow::Result<i32> {
    let count = EXPENSIVE_CALLS.fetch_add(1, Ordering::SeqCst) + 1;
    println!("expensive calculation running: call={}", count);
    tokio::time::sleep(Duration::from_secs(1)).await;
    Ok(value * value)
}
