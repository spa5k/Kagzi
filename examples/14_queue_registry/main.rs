use std::env;

use kagzi::Kagzi;
use serde_json::json;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let server = env::var("KAGZI_SERVER_URL").unwrap_or_else(|_| "http://localhost:50051".into());
    let namespace = env::var("KAGZI_NAMESPACE").unwrap_or_else(|_| "default".into());

    println!("🧾 Queue registry example");
    println!("  server    = {server}");
    println!("  namespace = {namespace}");

    let client = Kagzi::connect(&server).await?;

    // Create a queue (metadata-only). Safe to run multiple times: server returns conflict if it exists.
    let queue_name = env::var("KAGZI_QUEUE").unwrap_or_else(|_| "high_priority".into());

    match client
        .queue(&queue_name)
        .namespace(&namespace)
        .display_name("High Priority")
        .description("Latency-sensitive workflows")
        .label("lane", "realtime")
        .extra_json(json!({ "owner": "payments", "notes": "demo metadata" }))
        .send()
        .await
    {
        Ok(handle) => {
            println!(
                "✅ Created queue: {}/{}",
                handle.namespace, handle.task_queue
            );
        }
        Err(e) => {
            println!("⚠️  Create queue failed (likely already exists): {e}");
        }
    }

    // List queues
    let queues = client.list_queues(&namespace, None).await?;
    println!("📋 Queues in namespace '{namespace}':");
    for q in &queues {
        println!(
            "  - {} (enabled={}, display_name={})",
            q.task_queue,
            q.enabled,
            q.display_name.as_deref().unwrap_or("—")
        );
    }

    // Fetch the queue and toggle enabled (note: server disallows disabling 'default').
    if queue_name != "default" {
        let current = client.get_queue(&namespace, &queue_name).await?;
        let next_enabled = !current.enabled;

        let updated = client
            .update_queue(kagzi_proto::kagzi::UpdateQueueRequest {
                namespace: namespace.clone(),
                task_queue: queue_name.clone(),
                enabled: Some(next_enabled),
                display_name: None,
                description: None,
                labels: Default::default(),
                extra_json: None,
            })
            .await?;

        println!(
            "🔁 Toggled enabled: {} -> {}",
            current.enabled, updated.enabled
        );
    }

    println!("✅ Done");
    Ok(())
}
