//! Comprehensive example to exercise observability fields in workflow and step runs.
//!
//! This example demonstrates:
//! - Workflow `version` field (defaults to "1")
//! - Workflow `attempts` counter (incremented on pickup)
//! - Step `started_at` timestamp
//! - Step `input` recording
//! - Step `output` recording
//! - Step `finished_at` timestamp
//!
//! Run with:
//!   cargo run --example observability_demo
//!
//! Then check the database:
//!   psql -d kagzi -c "SELECT run_id, status, version, attempts, started_at, finished_at FROM kagzi.workflow_runs ORDER BY created_at DESC LIMIT 5;"
//!   psql -d kagzi -c "SELECT step_id, status, input, output, started_at, finished_at, attempt_number FROM kagzi.step_runs ORDER BY created_at DESC LIMIT 10;"

use kagzi::{Client, Worker, WorkflowContext};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize)]
struct UserData {
    user_id: String,
    email: String,
    plan: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct FetchUserInput {
    user_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct SendEmailInput {
    to: String,
    subject: String,
    template: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct EmailResult {
    message_id: String,
    sent_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct WorkflowOutput {
    user_id: String,
    emails_sent: u32,
    final_status: String,
}

/// Simulates fetching user data from a database
async fn fetch_user(input: &FetchUserInput) -> anyhow::Result<UserData> {
    println!("  📥 Fetching user: {}", input.user_id);
    tokio::time::sleep(Duration::from_millis(100)).await;
    Ok(UserData {
        user_id: input.user_id.clone(),
        email: format!("{}@example.com", input.user_id),
        plan: "premium".to_string(),
    })
}

/// Simulates sending an email
async fn send_email(input: &SendEmailInput) -> anyhow::Result<EmailResult> {
    println!("  📧 Sending email to {}: {}", input.to, input.subject);
    tokio::time::sleep(Duration::from_millis(50)).await;
    Ok(EmailResult {
        message_id: Uuid::new_v4().to_string(),
        sent_at: chrono::Utc::now().to_rfc3339(),
    })
}

/// Main workflow that demonstrates observability features
async fn user_onboarding_workflow(
    mut ctx: WorkflowContext,
    input: UserData,
) -> anyhow::Result<WorkflowOutput> {
    println!("\n🚀 Starting User Onboarding Workflow");
    println!("   Input: {:?}", input);

    // Step 1: Fetch user data with explicit input tracking
    let fetch_input = FetchUserInput {
        user_id: input.user_id.clone(),
    };
    let user = ctx
        .run_with_input("fetch_user_data", &fetch_input, fetch_user(&fetch_input))
        .await?;
    println!("   ✅ Fetched user: {:?}", user);

    // Step 2: Send welcome email with explicit input tracking
    let welcome_input = SendEmailInput {
        to: user.email.clone(),
        subject: "Welcome to our platform!".to_string(),
        template: "welcome_v2".to_string(),
    };
    let welcome_result = ctx
        .run_with_input(
            "send_welcome_email",
            &welcome_input,
            send_email(&welcome_input),
        )
        .await?;
    println!("   ✅ Welcome email sent: {}", welcome_result.message_id);

    // Step 3: Sleep to demonstrate sleep input recording
    println!("   ⏳ Waiting before sending tips email...");
    ctx.sleep(Duration::from_secs(2)).await?;

    // Step 4: Send tips email (this runs after wake-up)
    let tips_input = SendEmailInput {
        to: user.email.clone(),
        subject: "Tips for getting started".to_string(),
        template: "tips_v1".to_string(),
    };
    let tips_result = ctx
        .run_with_input("send_tips_email", &tips_input, send_email(&tips_input))
        .await?;
    println!("   ✅ Tips email sent: {}", tips_result.message_id);

    println!("🎉 Workflow completed successfully!\n");

    Ok(WorkflowOutput {
        user_id: user.user_id,
        emails_sent: 2,
        final_status: "onboarded".to_string(),
    })
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing for detailed logs
    tracing_subscriber::fmt()
        .with_env_filter("kagzi=info,observability_test=info")
        .init();

    let server_url =
        std::env::var("KAGZI_SERVER_URL").unwrap_or_else(|_| "http://localhost:50051".to_string());

    println!("╔════════════════════════════════════════════════════════════════╗");
    println!("║           Kagzi Observability Test Example                     ║");
    println!("╚════════════════════════════════════════════════════════════════╝");
    println!();
    println!("This example tests all the new observability fields:");
    println!("  • Workflow: version (default '1'), attempts, started_at");
    println!("  • Steps: input, output, started_at, finished_at, attempt_number");
    println!();
    println!("Connecting to server: {}", server_url);

    // Create worker and register workflow
    let mut worker = Worker::builder(&server_url, "observability-test-queue")
        .build()
        .await?;

    worker.register("UserOnboarding", user_onboarding_workflow);

    // Generate unique workflow ID for this test run
    let test_id = Uuid::new_v4().to_string()[..8].to_string();
    let idempotency_key = format!("obs-test-key-{}", test_id);

    println!("\n📋 Test Run Details:");
    println!("   Test ID: {}", test_id);
    println!("   Idempotency Key: {}", idempotency_key);

    // Create client to start workflow
    let mut client = Client::connect(&server_url).await?;

    // Start the workflow
    let input = UserData {
        user_id: format!("user-{}", test_id),
        email: format!("test-{}@example.com", test_id),
        plan: "premium".to_string(),
    };

    println!("\n▶ Starting workflow...");
    let run_id = client
        .workflow("UserOnboarding", "observability-test-queue", input)
        .idempotent(&idempotency_key)
        // Note: .version("X") can be added to set explicit version, otherwise defaults to "1"
        .await?;
    println!("   Run ID: {}", run_id);

    // Run the worker to execute the workflow
    println!("\n▶ Running worker (will process workflow and pause at sleep)...\n");

    // Run for a bit to process the workflow
    tokio::select! {
        result = worker.run() => {
            if let Err(e) = result {
                println!("Worker error: {}", e);
            }
        }
        _ = tokio::time::sleep(Duration::from_secs(30)) => {
            println!("\n⏰ Test timeout reached (30s)");
        }
    }

    println!("\n╔════════════════════════════════════════════════════════════════╗");
    println!("║                    Verification Queries                        ║");
    println!("╚════════════════════════════════════════════════════════════════╝");
    println!();
    println!("Run these SQL queries to verify the fields are populated:");
    println!();
    println!("-- Check workflow run (version, attempts, timestamps):");
    println!("SELECT run_id, workflow_type, status, version, attempts,");
    println!("       started_at, finished_at");
    println!("FROM kagzi.workflow_runs");
    println!("WHERE run_id = '{}'::uuid;", run_id);
    println!();
    println!("-- Check step runs (input, output, timestamps):");
    println!("SELECT step_id, status, attempt_number,");
    println!("       input::text as input,");
    println!("       output::text as output,");
    println!("       started_at, finished_at");
    println!("FROM kagzi.step_runs");
    println!("WHERE run_id = '{}'::uuid", run_id);
    println!("ORDER BY created_at ASC;");
    println!();
    println!("IMPORTANT NOTES:");
    println!("================");
    println!("• Workflow attempts = 1 for first pickup");
    println!("• Workflow attempts = 2 after sleep wakeup (picked up again)");
    println!("• Each step should have started_at and finished_at set");
    println!("• Steps with run_with_input() will have input recorded");
    println!("• Sleep steps automatically record duration as input");
    println!();

    Ok(())
}
