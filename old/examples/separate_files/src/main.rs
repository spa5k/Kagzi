use std::time::Duration;

use kagzi::Kagzi;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
struct User {
    id: String,
    name: String,
    email: String,
}

async fn create_user(name: String, email: String) -> anyhow::Result<User> {
    let user_id = format!("user_{}", &uuid::Uuid::new_v4().to_string()[..8]);
    let user = User {
        id: user_id.clone(),
        name,
        email,
    };

    println!("Created user: {} ({})", user.name, user.id);
    Ok(user)
}

async fn welcome_user_route(user_id: String, user_email: String) -> anyhow::Result<()> {
    let client = Kagzi::connect("http://localhost:50051").await?;

    println!("Triggering welcome email workflow for user: {}", user_id);

    // Import and use the input type
    use separate_files_example::types::SendWelcomeEmailInput;

    let input = SendWelcomeEmailInput {
        user_id: user_id.clone(),
        user_email,
    };

    // Run workflow asynchronously
    let run = client
        .start("send-welcome-email")
        .namespace("email")
        .input(&input)?
        .send()
        .await?;

    println!("Started welcome email workflow: {}", run.id);

    // Wait a bit to see the workflow complete
    tokio::time::sleep(Duration::from_secs(3)).await;

    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("🚀 Kagzi Example - Separate Files");
    println!("================================");

    // Create a test user
    let user = create_user("Alice Johnson".to_string(), "alice@example.com".to_string()).await?;

    // Trigger the welcome workflow
    welcome_user_route(user.id, user.email).await?;

    println!("\n✅ Workflow has been triggered!");
    println!("Make sure the worker is running to process it.");

    Ok(())
}
