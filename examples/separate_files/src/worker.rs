use kagzi::Worker;
use separate_files_example::workflows::send_welcome_email;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("🔄 Starting Kagzi worker...");

    let mut worker = Worker::new("http://localhost:50051")
        .namespace("email")
        .max_concurrent(10)
        .workflows([("send-welcome-email", send_welcome_email)])
        .build()
        .await?;

    // Start processing workflows
    println!("✅ Worker started. Listening for workflows on 'email' namespace...");
    println!("Press Ctrl+C to stop");

    worker.run().await?;

    Ok(())
}
