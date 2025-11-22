//! Example: Batch File Processing
//!
//! This example demonstrates:
//! - Processing items in batches
//! - Tracking progress across multiple batches
//! - Error handling per batch with continuation
//! - Aggregating results
//!
//! Run with: cargo run --example batch_processing

use kagzi::{Kagzi, WorkflowContext};
use serde::{Deserialize, Serialize};
use std::env;

#[derive(Debug, Serialize, Deserialize, Clone)]
struct BatchJobInput {
    job_id: String,
    file_paths: Vec<String>,
    batch_size: usize,
}

#[derive(Debug, Serialize, Deserialize)]
struct BatchResult {
    batch_number: usize,
    successful: usize,
    failed: usize,
    errors: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct JobResult {
    job_id: String,
    total_files: usize,
    successful_files: usize,
    failed_files: usize,
    batches_processed: usize,
    errors: Vec<String>,
}

/// Batch processing workflow
async fn batch_processing_workflow(
    ctx: WorkflowContext,
    input: BatchJobInput,
) -> anyhow::Result<JobResult> {
    println!("📁 Starting batch job: {}", input.job_id);
    println!("   Total files: {}", input.file_paths.len());
    println!("   Batch size: {}", input.batch_size);

    let total_files = input.file_paths.len();
    let mut total_successful = 0;
    let mut total_failed = 0;
    let mut all_errors = Vec::new();
    let mut batch_number = 0;

    // Process files in batches
    for chunk in input.file_paths.chunks(input.batch_size) {
        batch_number += 1;
        let batch_files = chunk.to_vec();

        // Each batch is a separate memoized step
        let step_id = format!("process-batch-{}", batch_number);
        let batch_result: BatchResult = ctx
            .step(&step_id, async move {
                println!(
                    "  📦 Processing batch {}/{} ({} files)...",
                    batch_number,
                    total_files.div_ceil(input.batch_size),
                    batch_files.len()
                );

                let mut successful = 0;
                let mut failed = 0;
                let mut errors = Vec::new();

                for file_path in &batch_files {
                    // Simulate file processing
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

                    // Simulate 10% failure rate
                    if file_path.contains("error") || (file_path.len() % 10 == 0) {
                        failed += 1;
                        let error_msg = format!("Failed to process: {}", file_path);
                        errors.push(error_msg.clone());
                        println!("     ✗ {}", error_msg);
                    } else {
                        successful += 1;
                        println!("     ✓ Processed: {}", file_path);
                    }
                }

                println!(
                    "     Batch {} complete: {} successful, {} failed",
                    batch_number, successful, failed
                );

                Ok::<_, anyhow::Error>(BatchResult {
                    batch_number,
                    successful,
                    failed,
                    errors,
                })
            })
            .await?;

        total_successful += batch_result.successful;
        total_failed += batch_result.failed;
        all_errors.extend(batch_result.errors);

        // Short delay between batches
        if batch_number < total_files.div_ceil(input.batch_size) {
            ctx.sleep(
                &format!("delay-after-batch-{}", batch_number),
                std::time::Duration::from_secs(1),
            )
            .await?;
        }
    }

    // Final summary step
    ctx.step("generate-summary-report", async {
        println!("  📊 Generating summary report...");

        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        println!("     ✓ Summary report generated");

        Ok::<_, anyhow::Error>(())
    })
    .await?;

    let result = JobResult {
        job_id: input.job_id.clone(),
        total_files,
        successful_files: total_successful,
        failed_files: total_failed,
        batches_processed: batch_number,
        errors: all_errors,
    };

    println!("\n✅ Batch job {} completed!", input.job_id);
    println!(
        "   Processed: {}/{} files successful ({:.1}% success rate)",
        result.successful_files,
        result.total_files,
        (result.successful_files as f64 / result.total_files as f64) * 100.0
    );

    Ok(result)
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
        .register_workflow("batch-processing", batch_processing_workflow)
        .await;

    // Generate sample file paths
    let file_paths: Vec<String> = (1..=25)
        .map(|i| format!("data/file_{:03}.csv", i))
        .collect();

    let batch_job = BatchJobInput {
        job_id: "JOB-2024-001".to_string(),
        file_paths,
        batch_size: 5, // Process 5 files at a time
    };

    println!("Starting batch processing workflow...");
    let handle = kagzi.start_workflow("batch-processing", batch_job).await?;

    println!("Workflow started: {}\n", handle.run_id());

    // Start worker in background
    let worker = kagzi.create_worker();
    tokio::spawn(async move {
        worker.start().await.expect("Worker failed");
    });

    // Wait for result
    println!("Processing batches...\n");
    let result_value = handle.result().await?;
    let result: JobResult = serde_json::from_value(result_value)?;
    println!("\n=== Batch Job Summary ===");
    println!("Job ID: {}", result.job_id);
    println!("Batches Processed: {}", result.batches_processed);
    println!("Total Files: {}", result.total_files);
    println!("Successful: {}", result.successful_files);
    println!("Failed: {}", result.failed_files);

    if !result.errors.is_empty() {
        println!("\nErrors:");
        for error in &result.errors {
            println!("  - {}", error);
        }
    }

    Ok(())
}
