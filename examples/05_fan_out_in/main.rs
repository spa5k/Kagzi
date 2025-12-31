use std::env;
use std::time::Duration;

use kagzi::WorkflowContext;
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
    let queue = env::var("KAGZI_TASK_QUEUE").unwrap_or_else(|_| "fanout".into());

    match variant {
        "static" => static_fanout(&server, &queue).await?,
        "mapreduce" => dynamic_mapreduce(&server, &queue).await?,
        _ => {
            eprintln!("Usage: cargo run -p kagzi --example 05_fan_out_in -- [static|mapreduce]");
            std::process::exit(1);
        }
    }

    Ok(())
}

async fn static_fanout(server: &str, queue: &str) -> anyhow::Result<()> {
    let mut worker = common::build_worker(server, queue).await?;
    worker.register("profile_aggregate", static_workflow);

    let mut client = common::connect_client(server).await?;
    let run = client
        .workflow(
            "profile_aggregate",
            queue,
            StaticInput {
                user_id: "user-123".into(),
            },
        )
        .await?;
    println!("Started static fan-out workflow: {}", run);

    tokio::spawn(async move { worker.run().await });
    tokio::time::sleep(Duration::from_secs(5)).await;
    Ok(())
}

async fn dynamic_mapreduce(server: &str, queue: &str) -> anyhow::Result<()> {
    let mut worker = common::build_worker(server, queue).await?;
    worker.register("square_and_sum", mapreduce_workflow);

    let mut client = common::connect_client(server).await?;
    let run = client
        .workflow(
            "square_and_sum",
            queue,
            NumbersInput {
                values: vec![1, 2, 3, 4],
            },
        )
        .await?;
    println!("Started dynamic map-reduce workflow: {}", run);

    tokio::spawn(async move { worker.run().await });
    tokio::time::sleep(Duration::from_secs(5)).await;
    Ok(())
}

async fn static_workflow(
    mut ctx: WorkflowContext,
    input: StaticInput,
) -> anyhow::Result<StaticOutput> {
    // Run sequentially to satisfy the single mutable borrow of `ctx`
    let user = ctx
        .run("fetch_user", fetch_user(input.user_id.clone()))
        .await?;
    let orders = ctx
        .run("fetch_orders", fetch_orders(input.user_id.clone()))
        .await?;
    let payments = ctx
        .run("fetch_payments", fetch_payments(input.user_id.clone()))
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
    mut ctx: WorkflowContext,
    input: NumbersInput,
) -> anyhow::Result<NumbersOutput> {
    // Run steps one by one (ctx is single-mut-borrow), but keep per-item step ids.
    let mut squared = Vec::with_capacity(input.values.len());
    for (idx, value) in input.values.iter().enumerate() {
        let val = ctx.run(&format!("square-{idx}"), square(*value)).await?;
        squared.push(val);
    }
    let sum = squared.iter().copied().sum();

    Ok(NumbersOutput { squared, sum })
}

async fn square(value: i32) -> anyhow::Result<i32> {
    sleep(Duration::from_millis(100)).await;
    Ok(value * value)
}
