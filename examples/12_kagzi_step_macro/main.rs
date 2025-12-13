//! Example demonstrating the `#[kagzi_step]` attribute macro.
//!
//! This example shows how the step macro enhances individual workflow step functions
//! with automatic tracing, logging, and error context.

use std::time::Duration;

use kagzi::prelude::*;
use kagzi_macros::kagzi_step;
use serde::{Deserialize, Serialize};

// Define input and output types
#[derive(Debug, Serialize, Deserialize, Clone)]
struct UserInput {
    user_id: String,
    email: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct UserProfile {
    user_id: String,
    email: String,
    tier: String,
    created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ValidationResult {
    is_valid: bool,
    user_profile: UserProfile,
}

// Regular step function without macro for comparison
async fn validate_user_regular(
    _ctx: WorkflowContext,
    input: UserInput,
) -> anyhow::Result<ValidationResult> {
    // Manual tracing and logging required
    let _span = tracing::info_span!("validate_user_regular");
    let _enter = _span.enter();

    tracing::debug!(input = ?input, "Starting user validation");

    // Validate input
    if input.user_id.is_empty() {
        tracing::error!("User ID cannot be empty");
        return Err(anyhow::anyhow!("User ID cannot be empty"));
    }

    if !input.email.contains('@') {
        tracing::error!(email = %input.email, "Invalid email format");
        return Err(anyhow::anyhow!("Invalid email format"));
    }

    // Simulate database lookup
    tokio::time::sleep(Duration::from_millis(100)).await;

    let profile = UserProfile {
        user_id: input.user_id.clone(),
        email: input.email.clone(),
        tier: "premium".to_string(),
        created_at: chrono::Utc::now(),
    };

    tracing::debug!(user_id = %input.user_id, "User validation completed");

    Ok(ValidationResult {
        is_valid: true,
        user_profile: profile,
    })
}

// Step function with #[kagzi_step] macro
#[kagzi_step]
/// Validates user input and fetches user profile
async fn validate_user_macro(
    _ctx: WorkflowContext,
    input: UserInput,
) -> anyhow::Result<ValidationResult> {
    // No manual tracing needed - macro handles it automatically!
    // Debug logs for input/output are added automatically
    // Error context is enriched automatically

    // Validate input
    if input.user_id.is_empty() {
        anyhow::bail!("User ID cannot be empty");
    }

    if !input.email.contains('@') {
        anyhow::bail!("Invalid email format: {}", input.email);
    }

    // Simulate database lookup
    tokio::time::sleep(Duration::from_millis(100)).await;

    let profile = UserProfile {
        user_id: input.user_id.clone(),
        email: input.email.clone(),
        tier: "premium".to_string(),
        created_at: chrono::Utc::now(),
    };

    Ok(ValidationResult {
        is_valid: true,
        user_profile: profile,
    })
}

#[kagzi_step]
/// Sends welcome email to user
async fn send_welcome_email(
    _ctx: WorkflowContext,
    user_profile: UserProfile,
) -> anyhow::Result<String> {
    // Simulate email sending with random failure
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Demonstrate error enrichment - the macro will add step context
    if user_profile.email.contains("fail") {
        anyhow::bail!("Failed to send email to {}", user_profile.email);
    }

    let email_id = format!("email-{}", uuid::Uuid::new_v4());

    tracing::info!(
        user_id = %user_profile.user_id,
        email = %user_profile.email,
        email_id = %email_id,
        "Welcome email sent"
    );

    Ok(email_id)
}

#[kagzi_step]
/// Updates user's onboarding status
async fn update_onboarding_status(_ctx: WorkflowContext, user_id: String) -> anyhow::Result<bool> {
    // Simulate database update
    tokio::time::sleep(Duration::from_millis(50)).await;

    tracing::info!(user_id = %user_id, "Updated onboarding status");

    Ok::<bool, anyhow::Error>(true)
}

// Note: Workflow that uses macro-enhanced steps
// In a real implementation, this would be registered with a Worker
// and executed through the Kagzi server.
// For this demo, we're just showing the syntax.

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    println!("🚀 Kagzi Step Macro Example");
    println!("============================");
    println!();
    println!("This example demonstrates the #[kagzi_step] attribute macro");
    println!("which automatically adds tracing, logging, and error context.");
    println!();

    // Test data
    let test_cases = vec![
        (
            "Valid User",
            UserInput {
                user_id: "user-123".to_string(),
                email: "alice@example.com".to_string(),
            },
        ),
        (
            "Invalid Email",
            UserInput {
                user_id: "user-456".to_string(),
                email: "invalid-email".to_string(),
            },
        ),
        (
            "Email Send Failure",
            UserInput {
                user_id: "user-789".to_string(),
                email: "fail@example.com".to_string(),
            },
        ),
    ];

    println!("Test cases:");
    for (name, input) in test_cases {
        println!("Testing: {}", name);
        println!("------------------------");

        // Note: In a real application, these functions would be called
        // within a WorkflowContext provided by the Kagzi runtime.
        // For this demo, we're just showing that the macros compile
        // and the syntax works correctly.

        println!("✅ Functions defined with #[kagzi_step]:");
        println!("  - validate_user_macro");
        println!("  - send_welcome_email");
        println!("  - update_onboarding_status");
        println!();
        println!("Each macro adds:");
        println!("  ✓ Automatic tracing spans");
        println!("  ✓ Input/output logging at debug level");
        println!("  ✓ Error context enrichment");
        println!();

        println!();
    }

    println!("Benefits of #[kagzi_step]:");
    println!("  ✓ Automatic tracing span with step name");
    println!("  ✓ Input/output logging at debug level");
    println!("  ✓ Error context enrichment with step information");
    println!("  ✓ No manual tracing code required");
    println!("  ✓ Preserves function behavior completely");
    println!();
    println!("To see debug logs, run with:");
    println!("  RUST_LOG=debug cargo run --example 12_kagzi_step_macro");

    Ok(())
}
