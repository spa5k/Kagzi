//! Example: Retry Pattern with Error Handling
//!
//! This example demonstrates:
//! - Implementing retry logic within workflow steps
//! - Handling transient failures
//! - Exponential backoff
//! - Graceful degradation
//!
//! Run with: cargo run --example retry_pattern

use kagzi::{Kagzi, WorkflowContext};
use serde::{Deserialize, Serialize};
use std::env;
use std::time::Duration;

#[derive(Debug, Serialize, Deserialize)]
struct ApiCallInput {
    endpoint: String,
    retry_attempts: u32,
}

#[derive(Debug, Serialize, Deserialize)]
struct ApiCallResult {
    success: bool,
    attempts_made: u32,
    response: Option<String>,
    error: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct WorkflowResult {
    api_calls_completed: usize,
    results: Vec<ApiCallResult>,
}

/// Simulate an unreliable API call
async fn simulate_unreliable_api_call(attempt: u32) -> Result<String, String> {
    println!("       Attempt {}: Calling API...", attempt);

    // Simulate network delay
    tokio::time::sleep(Duration::from_millis(300)).await;

    // Simulate 60% failure rate on first 2 attempts, then success
    if attempt < 3 && rand::random::<f64>() < 0.6 {
        Err(format!(
            "API Error: Connection timeout (attempt {})",
            attempt
        ))
    } else {
        Ok(format!(
            "{{\"status\":\"success\",\"data\":\"Result from attempt {}\"}}",
            attempt
        ))
    }
}

/// Retry with exponential backoff workflow
async fn retry_pattern_workflow(
    ctx: WorkflowContext,
    input: ApiCallInput,
) -> anyhow::Result<WorkflowResult> {
    println!("🔄 Starting retry pattern workflow");
    println!("   Endpoint: {}", input.endpoint);
    println!("   Max retry attempts: {}", input.retry_attempts);

    let mut results = Vec::new();

    // API Call 1: Critical operation with retries
    let result1: ApiCallResult = ctx
        .step("critical-api-call", async {
            println!("  📞 Making critical API call (with retries)...");

            let max_attempts = input.retry_attempts;
            let mut last_error = String::new();

            for attempt in 1..=max_attempts {
                match simulate_unreliable_api_call(attempt).await {
                    Ok(response) => {
                        println!("     ✓ API call succeeded on attempt {}", attempt);
                        return Ok::<_, anyhow::Error>(ApiCallResult {
                            success: true,
                            attempts_made: attempt,
                            response: Some(response),
                            error: None,
                        });
                    }
                    Err(err) => {
                        println!("     ✗ {}", err);
                        last_error = err;

                        if attempt < max_attempts {
                            // Exponential backoff: 1s, 2s, 4s, etc.
                            let backoff_secs = 2u64.pow(attempt - 1);
                            println!("       Waiting {} seconds before retry...", backoff_secs);
                            tokio::time::sleep(Duration::from_secs(backoff_secs)).await;
                        }
                    }
                }
            }

            // All retries exhausted
            println!("     ✗ All {} attempts failed", max_attempts);

            Ok::<_, anyhow::Error>(ApiCallResult {
                success: false,
                attempts_made: max_attempts,
                response: None,
                error: Some(last_error),
            })
        })
        .await?;

    results.push(result1);

    // Check if critical operation succeeded
    if results[0].success {
        // API Call 2: Non-critical operation (can fail gracefully)
        let result2: ApiCallResult = ctx
            .step("non-critical-api-call", async {
                println!("\n  📞 Making non-critical API call...");

                match simulate_unreliable_api_call(1).await {
                    Ok(response) => {
                        println!("     ✓ Non-critical API call succeeded");
                        Ok::<_, anyhow::Error>(ApiCallResult {
                            success: true,
                            attempts_made: 1,
                            response: Some(response),
                            error: None,
                        })
                    }
                    Err(err) => {
                        println!("     ⚠ Non-critical API call failed (continuing anyway)");
                        println!("       Error: {}", err);
                        Ok::<_, anyhow::Error>(ApiCallResult {
                            success: false,
                            attempts_made: 1,
                            response: None,
                            error: Some(err),
                        })
                    }
                }
            })
            .await?;

        results.push(result2);

        // API Call 3: With circuit breaker pattern
        let result3: ApiCallResult = ctx
            .step("circuit-breaker-api-call", async {
                println!("\n  📞 Making API call with circuit breaker pattern...");

                let mut consecutive_failures = 0;
                let circuit_breaker_threshold = 2;

                for attempt in 1..=3 {
                    match simulate_unreliable_api_call(attempt).await {
                        Ok(response) => {
                            println!("     ✓ API call succeeded");
                            return Ok::<_, anyhow::Error>(ApiCallResult {
                                success: true,
                                attempts_made: attempt,
                                response: Some(response),
                                error: None,
                            });
                        }
                        Err(_err) => {
                            consecutive_failures += 1;
                            println!("     ✗ Attempt {} failed", attempt);

                            if consecutive_failures >= circuit_breaker_threshold {
                                println!("     ⚠ Circuit breaker opened - too many failures");
                                return Ok::<_, anyhow::Error>(ApiCallResult {
                                    success: false,
                                    attempts_made: attempt,
                                    response: None,
                                    error: Some("Circuit breaker opened".to_string()),
                                });
                            }

                            tokio::time::sleep(Duration::from_secs(1)).await;
                        }
                    }
                }

                Ok::<_, anyhow::Error>(ApiCallResult {
                    success: false,
                    attempts_made: 3,
                    response: None,
                    error: Some("All attempts failed".to_string()),
                })
            })
            .await?;

        results.push(result3);
    } else {
        println!("\n  ⚠ Critical operation failed - skipping remaining steps");
    }

    // Summary step
    ctx.step("generate-summary", async {
        println!("\n  📊 Generating execution summary...");
        tokio::time::sleep(Duration::from_millis(200)).await;
        println!("     ✓ Summary generated");
        Ok::<_, anyhow::Error>(())
    })
    .await?;

    let workflow_result = WorkflowResult {
        api_calls_completed: results.len(),
        results,
    };

    println!("\n✅ Retry pattern workflow completed!");

    Ok(workflow_result)
}

// Simple random number generator for demo
mod rand {
    use std::cell::Cell;

    thread_local! {
        static SEED: Cell<u64> = Cell::new(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs()
        );
    }

    #[allow(private_bounds)]
    pub(crate) fn random<T: SampleUniform>() -> T {
        T::sample_single()
    }

    trait SampleUniform: Sized {
        fn sample_single() -> Self;
    }

    impl SampleUniform for f64 {
        fn sample_single() -> Self {
            SEED.with(|seed| {
                let mut s = seed.get();
                s ^= s << 13;
                s ^= s >> 7;
                s ^= s << 17;
                seed.set(s);
                (s as f64) / (u64::MAX as f64)
            })
        }
    }
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
        .register_workflow("retry-pattern", retry_pattern_workflow)
        .await;

    let input = ApiCallInput {
        endpoint: "https://api.example.com/resource".to_string(),
        retry_attempts: 5,
    };

    println!("Starting retry pattern workflow...\n");
    let handle = kagzi.start_workflow("retry-pattern", input).await?;

    println!("Workflow started: {}\n", handle.run_id());

    // Start worker in background
    let worker = kagzi.create_worker();
    tokio::spawn(async move {
        worker.start().await.expect("Worker failed");
    });

    // Wait for result
    let result_value = handle.result().await?;
    let result: WorkflowResult = serde_json::from_value(result_value)?;

    println!("\n=== Execution Summary ===");
    println!("API Calls Completed: {}", result.api_calls_completed);
    println!("\nResults:");
    for (idx, api_result) in result.results.iter().enumerate() {
        println!("\n  API Call {}:", idx + 1);
        println!("    Success: {}", api_result.success);
        println!("    Attempts: {}", api_result.attempts_made);
        if let Some(response) = &api_result.response {
            println!("    Response: {}", response);
        }
        if let Some(error) = &api_result.error {
            println!("    Error: {}", error);
        }
    }

    Ok(())
}
