use std::env;
use std::time::Duration;

use kagzi::{Context, Kagzi, Worker};
use serde::{Deserialize, Serialize};
use tokio::time::sleep;

#[path = "../common.rs"]
mod common;

#[derive(Debug, Serialize, Deserialize)]
struct StaticInput {
    user_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct StaticOutput {
    user: String,
    orders: Vec<String>,
    payments: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct NumbersInput {
    values: Vec<i32>,
}

#[derive(Debug, Serialize, Deserialize)]
struct NumbersOutput {
    squared: Vec<i32>,
    sum: i32,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = env::args().collect();
    let variant = args.get(1).map(|s| s.as_str()).unwrap_or("static");

    let server = env::var("KAGZI_SERVER_URL").unwrap_or_else(|_| "http://localhost:50051".into());
    let namespace = env::var("KAGZI_NAMESPACE").unwrap_or_else(|_| "fanout".into());

    match variant {
        "static" => static_fanout(&server, &namespace).await?,
        "mapreduce" => dynamic_mapreduce(&server, &namespace).await?,
        _ => {
            eprintln!("Usage: cargo run -p kagzi --example 05_fan_out_in -- [static|mapreduce]");
            std::process::exit(1);
        }
    }

    Ok(())
}

async fn static_fanout(server: &str, namespace: &str) -> anyhow::Result<()> {
    println!("🌐 Static Fan-Out Example - demonstrates parallel data fetching\n");

    let mut worker = Worker::new(server)
        .namespace(namespace)
        .workflows([("profile_aggregate", static_workflow)])
        .build()
        .await?;

    println!("👷 Worker started");

    let client = Kagzi::connect(server).await?;
    let input = StaticInput {
        user_id: "user-123".into(),
    };
    let run = client
        .start("profile_aggregate")
        .namespace(namespace)
        .input(&input)
        .send()
        .await?;
    println!("🚀 Started static fan-out workflow: {}", run.id);

    tokio::spawn(async move { worker.run().await });
    tokio::time::sleep(Duration::from_secs(5)).await;
    println!("✅ Example complete\n");
    Ok(())
}

async fn dynamic_mapreduce(server: &str, namespace: &str) -> anyhow::Result<()> {
    println!("🔢 Dynamic Map-Reduce Example - demonstrates parallel computation\n");

    let mut worker = Worker::new(server)
        .namespace(namespace)
        .workflows([("square_and_sum", mapreduce_workflow)])
        .build()
        .await?;

    println!("👷 Worker started");

    let client = Kagzi::connect(server).await?;
    let input = NumbersInput {
        values: vec![1, 2, 3, 4],
    };
    let run = client
        .start("square_and_sum")
        .namespace(namespace)
        .input(&input)
        .send()
        .await?;
    println!("🚀 Started dynamic map-reduce workflow: {}", run.id);

    tokio::spawn(async move { worker.run().await });
    tokio::time::sleep(Duration::from_secs(5)).await;
    println!("✅ Example complete\n");
    Ok(())
}

async fn static_workflow(mut ctx: Context, input: StaticInput) -> anyhow::Result<StaticOutput> {
    // Run sequentially to satisfy the single mutable borrow of `ctx`
    let user = ctx
        .step("fetch_user")
        .run(|| fetch_user(input.user_id.clone()))
        .await?;
    let orders = ctx
        .step("fetch_orders")
        .run(|| fetch_orders(input.user_id.clone()))
        .await?;
    let payments = ctx
        .step("fetch_payments")
        .run(|| fetch_payments(input.user_id.clone()))
        .await?;

    Ok(StaticOutput {
        user,
        orders,
        payments,
    })
}

async fn fetch_user(user_id: String) -> anyhow::Result<String> {
    sleep(Duration::from_millis(300)).await;
    Ok(format!("User<{user_id}>"))
}

async fn fetch_orders(user_id: String) -> anyhow::Result<Vec<String>> {
    sleep(Duration::from_millis(400)).await;
    Ok(vec![
        format!("order-1-for-{user_id}"),
        format!("order-2-for-{user_id}"),
    ])
}

async fn fetch_payments(user_id: String) -> anyhow::Result<Vec<String>> {
    sleep(Duration::from_millis(500)).await;
    Ok(vec![
        format!("payment-A-for-{user_id}"),
        format!("payment-B-for-{user_id}"),
    ])
}

async fn mapreduce_workflow(
    mut ctx: Context,
    input: NumbersInput,
) -> anyhow::Result<NumbersOutput> {
    // Run steps one by one (ctx is single-mut-borrow), but keep per-item step ids.
    let mut squared = Vec::with_capacity(input.values.len());
    for (idx, value) in input.values.iter().enumerate() {
        let val = ctx
            .step(format!("square-{idx}"))
            .run(|| square(*value))
            .await?;
        squared.push(val);
    }
    let sum = squared.iter().copied().sum();

    Ok(NumbersOutput { squared, sum })
}

async fn square(value: i32) -> anyhow::Result<i32> {
    sleep(Duration::from_millis(100)).await;
    Ok(value * value)
}
