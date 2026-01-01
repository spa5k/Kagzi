use std::env;
use std::time::Duration;

use kagzi::{Context, Kagzi, Worker};
use serde::{Deserialize, Serialize};
use tokio::task::JoinHandle;
use tokio::time::sleep;

#[path = "../common.rs"]
mod common;

#[derive(Debug, Serialize, Deserialize)]
struct TaskInput {
    label: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = env::args().collect();
    let variant = args.get(1).map(|s| s.as_str()).unwrap_or("priority");

    let server = env::var("KAGZI_SERVER_URL").unwrap_or_else(|_| "http://localhost:50051".into());

    match variant {
        "priority" => priority_demo(&server).await?,
        "namespace" => namespace_demo(&server).await?,
        _ => {
            eprintln!("Usage: cargo run -p kagzi --example 10_multi_queue -- [priority|namespace]");
            std::process::exit(1);
        }
    }

    Ok(())
}

async fn priority_demo(server: &str) -> anyhow::Result<()> {
    // High and low priority queues handled by different workers in same process
    let mut high_worker = Worker::new(server)
        .namespace("high-priority")
        .workflows([
            ("priority_task", |_: Context, input: TaskInput| async move {
                println!("handling priority task: queue=high, label={}", input.label);
                sleep(Duration::from_secs(1)).await;
                Ok(serde_json::json!({"queue": "high", "label": input.label}))
            }),
        ])
        .build()
        .await?;

    let mut low_worker = Worker::new(server)
        .namespace("low-priority")
        .workflows([
            ("priority_task", |_: Context, input: TaskInput| async move {
                println!("handling low task: queue=low, label={}", input.label);
                sleep(Duration::from_secs(2)).await;
                Ok(serde_json::json!({"queue": "low", "label": input.label}))
            }),
        ])
        .build()
        .await?;

    let client = Kagzi::connect(server).await?;
    let mut handles: Vec<JoinHandle<anyhow::Result<()>>> = Vec::new();
    handles.push(tokio::spawn(async move { high_worker.run().await }));
    handles.push(tokio::spawn(async move { low_worker.run().await }));

    let high = client
        .start("priority_task")
        .namespace("high-priority")
        .input(TaskInput {
            label: "urgent-report".into(),
        })
        .send()
        .await?;
    let low = client
        .start("priority_task")
        .namespace("low-priority")
        .input(TaskInput {
            label: "weekly-digest".into(),
        })
        .send()
        .await?;

    println!(
        "High queue should finish first: high={}, low={}",
        high.id, low.id
    );
    tokio::time::sleep(Duration::from_secs(6)).await;
    for h in handles {
        h.abort();
    }
    Ok(())
}

async fn namespace_demo(server: &str) -> anyhow::Result<()> {
    // Simulate namespace isolation via separate namespaces
    let mut prod_worker = Worker::new(server)
        .namespace("production")
        .workflows([("ns_task", |_: Context, input: TaskInput| async move {
            println!(
                "processing prod task: namespace=production, label={}",
                input.label
            );
            Ok(serde_json::json!({"ns": "prod", "label": input.label}))
        })])
        .build()
        .await?;

    let mut staging_worker = Worker::new(server)
        .namespace("staging")
        .workflows([("ns_task", |_: Context, input: TaskInput| async move {
            println!(
                "processing staging task: namespace=staging, label={}",
                input.label
            );
            Ok(serde_json::json!({"ns": "staging", "label": input.label}))
        })])
        .build()
        .await?;

    let client = Kagzi::connect(server).await?;
    let mut handles: Vec<JoinHandle<anyhow::Result<()>>> = Vec::new();
    handles.push(tokio::spawn(async move { prod_worker.run().await }));
    handles.push(tokio::spawn(async move { staging_worker.run().await }));

    let prod_run = client
        .start("ns_task")
        .namespace("production")
        .input(TaskInput {
            label: "prod-task".into(),
        })
        .send()
        .await?;
    let staging_run = client
        .start("ns_task")
        .namespace("staging")
        .input(TaskInput {
            label: "staging-task".into(),
        })
        .send()
        .await?;

    println!(
        "Namespaces isolate tenants: prod_run={}, staging_run={}",
        prod_run.id, staging_run.id
    );
    tokio::time::sleep(Duration::from_secs(5)).await;
    for h in handles {
        h.abort();
    }
    Ok(())
}
