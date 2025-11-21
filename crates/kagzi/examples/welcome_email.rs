//! Example: Welcome Email Workflow
//!
//! This example demonstrates:
//! - Defining a workflow with multiple steps
//! - Step memoization (steps don't re-run on restart)
//! - Durable sleep
//! - Starting workflows from a client
//!
//! To run this example:
//! 1. Start PostgreSQL: `docker-compose up -d`
//! 2. Run the worker: `cargo run --example welcome_email -- worker`
//! 3. In another terminal, trigger a workflow: `cargo run --example welcome_email -- trigger Alice`

use kagzi::{Kagzi, WorkflowContext};
use serde::{Deserialize, Serialize};
use std::env;
use std::time::Duration;

#[derive(Debug, Serialize, Deserialize)]
struct WelcomeInput {
    user_name: String,
    user_email: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct User {
    id: String,
    name: String,
    email: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct WelcomeResult {
    user_id: String,
    email_sent: bool,
}

/// The welcome email workflow
async fn welcome_email_workflow(
    ctx: WorkflowContext,
    input: WelcomeInput,
) -> anyhow::Result<WelcomeResult> {
    println!(
        "🚀 Starting welcome email workflow for: {}",
        input.user_name
    );

    // Step 1: Create user in database (memoized)
    let user: User = ctx
        .step("create-user", async {
            println!("  📝 Creating user in database...");
            // Simulate database operation
            tokio::time::sleep(Duration::from_secs(1)).await;

            Ok::<_, anyhow::Error>(User {
                id: format!("user_{}", uuid::Uuid::new_v4()),
                name: input.user_name.clone(),
                email: input.user_email.clone(),
            })
        })
        .await?;

    println!("  ✅ User created: {}", user.id);

    // Step 2: Wait before sending email (durable sleep)
    println!("  ⏰ Sleeping for 5 seconds...");
    ctx.sleep("wait-before-email", Duration::from_secs(5))
        .await?;

    println!("  ✅ Woke up from sleep!");

    // Step 3: Send welcome email (memoized)
    let email_sent: bool = ctx
        .step("send-welcome-email", async {
            println!("  📧 Sending welcome email to {}...", user.email);
            // Simulate email sending
            tokio::time::sleep(Duration::from_secs(1)).await;

            Ok::<_, anyhow::Error>(true)
        })
        .await?;

    println!("  ✅ Email sent!");

    // Step 4: Mark user as onboarded (memoized)
    ctx.step("mark-onboarded", async {
        println!("  🎉 Marking user as onboarded...");
        tokio::time::sleep(Duration::from_millis(500)).await;

        Ok::<_, anyhow::Error>(())
    })
    .await?;

    println!("✅ Welcome workflow completed for user: {}", user.id);

    Ok(WelcomeResult {
        user_id: user.id,
        email_sent,
    })
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter("info,kagzi=debug,sqlx=warn")
        .init();

    let database_url = env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://kagzi:kagzi_dev_password@localhost:5432/kagzi".to_string());

    let kagzi = Kagzi::connect(&database_url).await?;

    // Register the workflow
    kagzi
        .register_workflow("welcome-email", welcome_email_workflow)
        .await;

    let args: Vec<String> = env::args().collect();

    if args.len() < 2 {
        eprintln!("Usage:");
        eprintln!("  cargo run --example welcome_email -- worker          # Start a worker");
        eprintln!("  cargo run --example welcome_email -- trigger <name>  # Trigger a workflow");
        std::process::exit(1);
    }

    match args[1].as_str() {
        "worker" => {
            println!("🔧 Starting worker...");
            let worker = kagzi.create_worker();
            worker.start().await?;
        }
        "trigger" => {
            if args.len() < 3 {
                eprintln!("Error: Please provide a name");
                eprintln!("Usage: cargo run --example welcome_email -- trigger <name>");
                std::process::exit(1);
            }

            let name = &args[2];
            println!("🚀 Triggering workflow for: {}", name);

            let handle = kagzi
                .start_workflow(
                    "welcome-email",
                    WelcomeInput {
                        user_name: name.to_string(),
                        user_email: format!("{}@example.com", name.to_lowercase()),
                    },
                )
                .await?;

            println!("✅ Workflow started!");
            println!("   Run ID: {}", handle.run_id());
            println!("   Make sure a worker is running to execute it.");
        }
        _ => {
            eprintln!("Unknown command: {}", args[1]);
            eprintln!("Use 'worker' or 'trigger <name>'");
            std::process::exit(1);
        }
    }

    Ok(())
}
