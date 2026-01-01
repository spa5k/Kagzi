use kagzi::Kagzi;
use separate_files_example::types::SendWelcomeEmailInput;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("🎯 Triggering a single workflow...");

    let client = Kagzi::connect("http://localhost:50051").await?;

    let run = client
        .start("send-welcome-email")
        .namespace("email")
        .input(SendWelcomeEmailInput {
            user_id: "123".to_string(),
            user_email: "alice@example.com".to_string(),
        })
        .send()
        .await?;

    println!("✅ Started welcome email workflow: {}", run.id);
    Ok(())
}
