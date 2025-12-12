use kagzi::Client;
use separate_files_example::types::SendWelcomeEmailInput;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("🎯 Triggering a single workflow...");

    let mut client = Client::connect("http://localhost:50051").await?;

    let run_id = client
        .workflow(
            "send-welcome-email",
            "email",
            SendWelcomeEmailInput {
                user_id: "123".to_string(),
                user_email: "alice@example.com".to_string(),
            },
        )
        .retries(3)
        .await?;

    println!("✅ Started welcome email workflow: {}", run_id);
    Ok(())
}
