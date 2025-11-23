//! Example: Step-Level Retry Policies
//!
//! This example demonstrates V2 retry policy features:
//! - Configuring retry policies per step
//! - Exponential backoff with jitter
//! - Fixed interval retries
//! - Retry predicates and error classification
//!
//! Run with: cargo run --example step_retry_policies

use kagzi::{Kagzi, RetryPolicy, RetryPredicate, StepBuilder, WorkflowContext};
use kagzi_core::{ErrorKind, StepError};
use serde::{Deserialize, Serialize};
use std::env;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tracing::info;

#[derive(Debug, Serialize, Deserialize)]
struct RetryDemoInput {
    user_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct RetryDemoOutput {
    user_id: String,
    api_call_attempts: u32,
    db_operation_attempts: u32,
    status: String,
}

/// Simulate an unreliable API that succeeds after a few retries
async fn unreliable_api_call(attempt_counter: Arc<AtomicU32>) -> Result<String, StepError> {
    let attempt = attempt_counter.fetch_add(1, Ordering::SeqCst) + 1;
    info!("      API attempt #{}", attempt);

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Succeed on 3rd attempt
    if attempt >= 3 {
        info!("      ✓ API call succeeded!");
        Ok("API response data".to_string())
    } else {
        info!("      ✗ API call failed (network error)");
        // Return a retryable network error
        Err(StepError::new(
            ErrorKind::NetworkError,
            "Connection timeout",
        ))
    }
}

/// Simulate a database operation that might fail temporarily
async fn unreliable_db_operation(attempt_counter: Arc<AtomicU32>) -> Result<(), StepError> {
    let attempt = attempt_counter.fetch_add(1, Ordering::SeqCst) + 1;
    info!("      DB attempt #{}", attempt);

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Succeed on 2nd attempt
    if attempt >= 2 {
        info!("      ✓ Database operation succeeded!");
        Ok(())
    } else {
        info!("      ✗ Database operation failed (temporary error)");
        Err(StepError::new(
            ErrorKind::DatabaseError,
            "Temporary database connection issue",
        ))
    }
}

/// Workflow demonstrating different retry policies
async fn retry_policies_workflow(
    ctx: WorkflowContext,
    input: RetryDemoInput,
) -> anyhow::Result<RetryDemoOutput> {
    info!("🔄 Starting retry policies demonstration");
    info!("   User ID: {}\n", input.user_id);

    // Step 1: API call with exponential backoff retry policy
    info!("📞 Step 1: API call with exponential backoff");
    info!("   Policy: 5 attempts, 1s initial delay, 2x multiplier");

    let api_attempt_counter = Arc::new(AtomicU32::new(0));
    let api_response: String = StepBuilder::new("fetch-user-data")
        .retry_policy(RetryPolicy::exponential_with(
            1000,    // 1 second initial delay
            30000,   // 30 second max delay
            2.0,     // Double the delay each time
            5,       // Max 5 attempts
            true,    // Use jitter
        ))
        .retry_predicate(RetryPredicate::OnRetryableError)
        .execute(&ctx, {
            let counter = api_attempt_counter.clone();
            async move { unreliable_api_call(counter).await.map_err(|e| anyhow::anyhow!(e)) }
        })
        .await?;

    let api_attempts = api_attempt_counter.load(Ordering::SeqCst);
    info!("   ✓ Completed after {} attempts\n", api_attempts);

    // Step 2: Database operation with fixed interval retry
    info!("💾 Step 2: Database operation with fixed interval retry");
    info!("   Policy: 3 attempts, 2s fixed interval");

    let db_attempt_counter = Arc::new(AtomicU32::new(0));
    StepBuilder::new("save-user-data")
        .retry_policy(RetryPolicy::fixed_with(
            2000, // 2 second delay between retries
            3,    // Max 3 attempts
        ))
        .retry_predicate(RetryPredicate::OnErrorKind(vec![
            ErrorKind::DatabaseError,
            ErrorKind::Timeout,
        ]))
        .execute(&ctx, {
            let counter = db_attempt_counter.clone();
            async move { unreliable_db_operation(counter).await.map_err(|e| anyhow::anyhow!(e)) }
        })
        .await?;

    let db_attempts = db_attempt_counter.load(Ordering::SeqCst);
    info!("   ✓ Completed after {} attempts\n", db_attempts);

    // Step 3: Critical operation with no retry (fail immediately)
    info!("⚡ Step 3: Critical operation (no retry - fail fast)");
    info!("   Policy: No retries");

    StepBuilder::new("critical-validation")
        .retry_policy(RetryPolicy::none())
        .execute(&ctx, async {
            info!("      Performing critical validation...");
            tokio::time::sleep(Duration::from_millis(100)).await;
            info!("      ✓ Validation passed!");
            Ok::<_, anyhow::Error>(())
        })
        .await?;

    info!("   ✓ Completed\n");

    // Step 4: Notification with retry on any error
    info!("📧 Step 4: Notification with retry on any error");
    info!("   Policy: 3 attempts, exponential backoff, retry all errors");

    let notification_counter = Arc::new(AtomicU32::new(0));
    StepBuilder::new("send-notification")
        .retry_policy(RetryPolicy::exponential_with(
            500,   // 500ms initial
            10000, // 10s max
            2.0,   // Double each time
            3,     // 3 attempts
            false, // No jitter for demo clarity
        ))
        .retry_predicate(RetryPredicate::OnAnyError) // Retry all errors
        .execute(&ctx, {
            let counter = notification_counter.clone();
            async move {
                let attempt = counter.fetch_add(1, Ordering::SeqCst) + 1;
                info!("      Notification attempt #{}", attempt);

                if attempt >= 2 {
                    info!("      ✓ Notification sent!");
                    Ok::<_, anyhow::Error>(())
                } else {
                    info!("      ✗ Notification failed");
                    Err(anyhow::anyhow!("Email service unavailable"))
                }
            }
        })
        .await?;

    info!("   ✓ Completed\n");

    let output = RetryDemoOutput {
        user_id: input.user_id,
        api_call_attempts: api_attempts,
        db_operation_attempts: db_attempts,
        status: "completed".to_string(),
    };

    info!("✅ Retry policies workflow completed!");

    Ok(output)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("info,kagzi=debug,sqlx=warn")
        .init();

    let database_url = env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://kagzi:kagzi_dev_password@localhost:5432/kagzi".to_string());

    let kagzi = Kagzi::connect(&database_url).await?;

    kagzi
        .register_workflow("retry-policies", retry_policies_workflow)
        .await;

    let input = RetryDemoInput {
        user_id: "user-123".to_string(),
    };

    info!("Starting retry policies demonstration...\n");
    let handle = kagzi.start_workflow("retry-policies", input).await?;
    info!("Workflow started: {}\n", handle.run_id());

    // Start worker
    let worker = kagzi.create_worker();
    tokio::spawn(async move {
        worker.start().await.expect("Worker failed");
    });

    // Wait for result
    let result_value = handle.result().await?;
    let result: RetryDemoOutput = serde_json::from_value(result_value)?;

    info!("\n=== Execution Summary ===");
    info!("User ID: {}", result.user_id);
    info!("Status: {}", result.status);
    info!("\nRetry Statistics:");
    info!("  • API call: {} attempts", result.api_call_attempts);
    info!("  • DB operation: {} attempts", result.db_operation_attempts);

    info!("\n💡 Key Features Demonstrated:");
    info!("  • Exponential backoff with configurable multiplier and jitter");
    info!("  • Fixed interval retries");
    info!("  • No retry (fail-fast) policy");
    info!("  • Retry predicates (OnRetryableError, OnErrorKind, OnAnyError)");
    info!("  • Step-level retry configuration with StepBuilder");
    info!("  • Automatic retry handling with configurable delays");

    Ok(())
}
