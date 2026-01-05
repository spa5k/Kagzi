use std::env;
use std::time::Duration;

use kagzi::{Context, Kagzi, Worker};
use serde::{Deserialize, Serialize};
use tokio::time::sleep;

#[path = "../common.rs"]
mod common;

#[derive(Debug, Serialize, Deserialize)]
struct TransformInput {
    payload: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize)]
struct LargeInput {
    content: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct LargeOutput {
    blob_key: String,
    size: usize,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = env::args().collect();
    let variant = args.get(1).map(|s| s.as_str()).unwrap_or("transform");

    let server = env::var("KAGZI_SERVER_URL").unwrap_or_else(|_| "http://localhost:50051".into());
    let namespace = env::var("KAGZI_NAMESPACE").unwrap_or_else(|_| "data-pipeline".into());

    match variant {
        "transform" => transform_demo(&server, &namespace).await?,
        "large" => large_payload_demo(&server, &namespace).await?,
        _ => {
            eprintln!("Usage: cargo run -p kagzi --example 09_data_pipeline -- [transform|large]");
            std::process::exit(1);
        }
    }

    Ok(())
}

async fn transform_demo(server: &str, namespace: &str) -> anyhow::Result<()> {
    println!("🔧 Transform Example - demonstrates JSON payload transformation\n");

    let mut worker = Worker::new(server)
        .namespace(namespace)
        .workflows([("json_transform", transform_workflow)])
        .build()
        .await?;

    println!("👷 Worker started");

    let client = Kagzi::connect(server).await?;
    let input = TransformInput {
        payload: serde_json::json!({ "name": "kagzi", "version": 1 }),
    };
    let run = client
        .start("json_transform")
        .namespace(namespace)
        .input(&input)
        .send()
        .await?;

    println!("🚀 Started pass-through transform workflow: {}", run.id);
    tokio::spawn(async move { worker.run().await });
    tokio::time::sleep(Duration::from_secs(5)).await;
    println!("✅ Example complete\n");
    Ok(())
}

async fn large_payload_demo(server: &str, namespace: &str) -> anyhow::Result<()> {
    println!("📦 Large Payload Example - demonstrates blob storage for big data\n");

    let blob = common::InMemoryBlobStore::new();
    let mut worker = Worker::new(server)
        .namespace(namespace)
        .workflows([(
            "large_payload",
            move |mut ctx: Context, input: LargeInput| {
                let blob = blob.clone();
                async move {
                    // Step 1: offload large payload
                    let key = ctx
                        .step("store")
                        .run(|| store_large(blob.clone(), input.content.clone()))
                        .await?;
                    // Step 2: download
                    let content = ctx
                        .step("load")
                        .run(|| load_large(blob.clone(), key.clone()))
                        .await?;

                    Ok(LargeOutput {
                        blob_key: key,
                        size: content.len(),
                    })
                }
            },
        )])
        .build()
        .await?;

    println!("👷 Worker started");

    let client = Kagzi::connect(server).await?;
    let big = "x".repeat(1_200_000); // 1.2MB to trigger warning thresholds
    let input = LargeInput { content: big };
    let run = client
        .start("large_payload")
        .namespace(namespace)
        .input(&input)
        .send()
        .await?;

    println!(
        "🚀 Started large payload workflow (stores data in mock blob): {}",
        run.id
    );
    tokio::spawn(async move { worker.run().await });
    tokio::time::sleep(Duration::from_secs(6)).await;
    println!("✅ Example complete\n");
    Ok(())
}

async fn transform_workflow(
    mut ctx: Context,
    input: TransformInput,
) -> anyhow::Result<serde_json::Value> {
    let uppercased = ctx
        .step("upper-name")
        .run(|| uppercase_name(input.payload.clone()))
        .await?;
    let enriched = ctx
        .step("enrich")
        .run(|| enrich_payload(uppercased.clone()))
        .await?;
    Ok(enriched)
}

async fn uppercase_name(mut payload: serde_json::Value) -> anyhow::Result<serde_json::Value> {
    if let Some(name) = payload.get_mut("name")
        && let Some(s) = name.as_str()
    {
        *name = serde_json::Value::String(s.to_uppercase());
    }
    sleep(Duration::from_millis(100)).await;
    Ok(payload)
}

async fn enrich_payload(payload: serde_json::Value) -> anyhow::Result<serde_json::Value> {
    let mut enriched = payload;
    enriched["processed_at"] = serde_json::Value::String(chrono::Utc::now().to_rfc3339());
    sleep(Duration::from_millis(100)).await;
    Ok(enriched)
}

async fn store_large(blob: common::InMemoryBlobStore, content: String) -> anyhow::Result<String> {
    let bytes = content.into_bytes();
    let key = blob.put(bytes).await;
    println!("stored payload in blob store (mock): {}", key);
    Ok(key)
}

async fn load_large(blob: common::InMemoryBlobStore, key: String) -> anyhow::Result<String> {
    let data = blob
        .get(&key)
        .await
        .ok_or_else(|| anyhow::anyhow!("missing key {key}"))?;
    println!("loaded payload from blob store (mock): {}", key);
    String::from_utf8(data).map_err(|e| anyhow::anyhow!("invalid UTF-8 in blob {key}: {e}"))
}
