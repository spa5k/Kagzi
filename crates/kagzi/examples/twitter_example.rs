use kagzi::{Client, Worker, WorkflowContext};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::info;
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize)]
struct Input {
    email: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct Output {
    user_id: String,
    status: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct User {
    id: String,
    email: String,
}

async fn user_signup(mut ctx: WorkflowContext, input: Input) -> anyhow::Result<Output> {
    info!("Starting user signup workflow for: {}", input.email);

    let user = ctx.run("create_user", create_user(input.email)).await?;

    ctx.run("send_welcome_email", send_welcome_email(user.clone()))
        .await?;

    info!("Sleeping for 10 seconds...");
    ctx.sleep(Duration::from_secs(10)).await?;
    info!("Resumed after sleep");

    ctx.run("send_review_email", send_review_email(user.clone()))
        .await?;

    info!("Workflow completed successfully!");
    Ok(Output {
        user_id: user.id,
        status: "onboarded".to_string(),
    })
}

async fn create_user(email: String) -> anyhow::Result<User> {
    info!("Creating user with email: {}", email);
    let user = User {
        id: Uuid::new_v4().to_string(),
        email,
    };
    info!("Created user: {}", user.id);
    Ok(user)
}

async fn send_welcome_email(user: User) -> anyhow::Result<()> {
    info!("Sending welcome email to user: {}", user.id);
    Ok(())
}

async fn send_review_email(user: User) -> anyhow::Result<()> {
    if !user.email.contains('@') {
        return Err(anyhow::anyhow!("Invalid email"));
    }
    info!("Sending review email to user: {}", user.id);
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let mut worker = Worker::new("http://localhost:50051", "queue").await?;
    worker.register("UserSignup", user_signup);

    tokio::spawn(async move {
        let _ = worker.run().await;
    });

    tokio::time::sleep(Duration::from_millis(500)).await;

    let mut client = Client::connect("http://localhost:50051").await?;

    // Simple - just await directly
    let run_id = client
        .workflow(
            "UserSignup",
            "queue",
            Input {
                email: "user@example.com".to_string(),
            },
        )
        .await?;

    info!("Started workflow: {}", run_id);

    // With options - chain what you need, then await
    let run_id = client
        .workflow(
            "UserSignup",
            "queue",
            Input {
                email: "user@example.com".to_string(),
            },
        )
        .idempotent(format!("user-signup-{}", Uuid::new_v4()))
        .await?;

    info!("Started idempotent workflow: {}", run_id);

    tokio::time::sleep(Duration::from_secs(20)).await;

    Ok(())
}
