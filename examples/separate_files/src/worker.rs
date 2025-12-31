use kagzi::Worker;
use separate_files_example::workflows::send_welcome_email;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("🔄 Starting Kagzi worker...");

    let mut worker = Worker::builder("http://localhost:50051", "email")
        .max_concurrent(10)
        .build()
        .await?;

    // Register all workflows
    worker.register("send-welcome-email", send_welcome_email);

    // Start processing workflows
    println!("✅ Worker started. Listening for workflows on 'email' queue...");
    println!("Press Ctrl+C to stop");

    worker.run().await?;

    Ok(())
}
