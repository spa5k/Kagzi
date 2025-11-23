//! Example: Advanced Parallel Execution
//!
//! This example demonstrates V2 parallel execution features:
//! - Parallel step execution with ParallelExecutor
//! - Different error handling strategies (FailFast vs CollectAll)
//! - Memoization in parallel steps
//! - Mixed parallel and sequential execution
//!
//! Run with: cargo run --example advanced_parallel

use kagzi::{Kagzi, ParallelErrorStrategy, ParallelExecutor, WorkflowContext};
use serde::{Deserialize, Serialize};
use std::env;
use std::time::Duration;
use tracing::info;
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize)]
struct DataProcessingInput {
    dataset_ids: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct DatasetResult {
    dataset_id: String,
    records_processed: i32,
    processing_time_ms: u64,
}

#[derive(Debug, Serialize, Deserialize)]
struct ProcessingOutput {
    total_datasets: usize,
    successful_datasets: usize,
    failed_datasets: usize,
    results: Vec<DatasetResult>,
    total_records: i32,
}

/// Simulate processing a dataset
async fn process_dataset(dataset_id: String) -> anyhow::Result<DatasetResult> {
    info!("    Processing dataset: {}", dataset_id);

    let start = std::time::Instant::now();

    // Simulate varying processing times
    let delay_ms = match dataset_id.as_str() {
        id if id.contains("fast") => 500,
        id if id.contains("slow") => 2000,
        _ => 1000,
    };

    tokio::time::sleep(Duration::from_millis(delay_ms)).await;

    // Simulate occasional failures
    if dataset_id.contains("error") {
        return Err(anyhow::anyhow!("Failed to process dataset: {}", dataset_id));
    }

    let records = match dataset_id.as_str() {
        id if id.contains("large") => 10000,
        id if id.contains("small") => 100,
        _ => 1000,
    };

    Ok(DatasetResult {
        dataset_id,
        records_processed: records,
        processing_time_ms: start.elapsed().as_millis() as u64,
    })
}

/// Advanced parallel processing workflow
async fn parallel_data_processing(
    ctx: WorkflowContext,
    input: DataProcessingInput,
) -> anyhow::Result<ProcessingOutput> {
    info!(
        "📊 Starting parallel data processing for {} datasets",
        input.dataset_ids.len()
    );

    // Phase 1: Parallel preprocessing with FailFast strategy
    info!("\n🔄 Phase 1: Preprocessing datasets (FailFast strategy)");
    let preprocessing_group = Uuid::new_v4();
    let preprocessor = ParallelExecutor::new(
        &ctx,
        preprocessing_group,
        Some("preprocess-phase".to_string()),
        ParallelErrorStrategy::FailFast,
    );

    let mut preprocess_handles = vec![];
    for dataset_id in &input.dataset_ids {
        let dataset_id = dataset_id.clone();
        let handle = tokio::spawn({
            let dataset_id = dataset_id.clone();
            let step_id = format!("preprocess-{}", dataset_id);
            async move {
                preprocessor
                    .execute_step(&step_id, async {
                        info!("    Preprocessing: {}", dataset_id);
                        tokio::time::sleep(Duration::from_millis(300)).await;
                        Ok::<_, anyhow::Error>(dataset_id)
                    })
                    .await
            }
        });
        preprocess_handles.push(handle);
    }

    // Wait for all preprocessing to complete (FailFast on error)
    let preprocessed_datasets: Vec<String> = futures::future::try_join_all(preprocess_handles)
        .await?
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?;

    info!("  ✓ Preprocessing completed for {} datasets", preprocessed_datasets.len());

    // Phase 2: Parallel processing with CollectAll strategy (tolerate failures)
    info!("\n🔄 Phase 2: Processing datasets (CollectAll strategy - tolerate failures)");
    let processing_group = Uuid::new_v4();
    let processor = ParallelExecutor::new(
        &ctx,
        processing_group,
        Some("process-phase".to_string()),
        ParallelErrorStrategy::CollectAll,
    );

    let mut process_handles = vec![];
    for dataset_id in preprocessed_datasets {
        let handle = tokio::spawn({
            let dataset_id = dataset_id.clone();
            let step_id = format!("process-{}", dataset_id);
            async move {
                processor
                    .execute_step(&step_id, process_dataset(dataset_id))
                    .await
            }
        });
        process_handles.push(handle);
    }

    // Collect all results, including errors
    let process_results = futures::future::join_all(process_handles).await;

    let mut successful_results = vec![];
    let mut failed_count = 0;

    for result in process_results {
        match result {
            Ok(Ok(dataset_result)) => {
                info!(
                    "  ✓ Processed: {} ({} records in {}ms)",
                    dataset_result.dataset_id,
                    dataset_result.records_processed,
                    dataset_result.processing_time_ms
                );
                successful_results.push(dataset_result);
            }
            Ok(Err(e)) => {
                info!("  ✗ Processing failed: {}", e);
                failed_count += 1;
            }
            Err(e) => {
                info!("  ✗ Task failed: {}", e);
                failed_count += 1;
            }
        }
    }

    // Phase 3: Sequential aggregation
    info!("\n🔄 Phase 3: Aggregating results (sequential)");
    let total_records: i32 = ctx
        .step("aggregate-results", async {
            let total = successful_results
                .iter()
                .map(|r| r.records_processed)
                .sum();
            info!("  ✓ Total records processed: {}", total);
            Ok::<_, anyhow::Error>(total)
        })
        .await?;

    // Phase 4: Generate report
    ctx.step("generate-report", async {
        info!("  ✓ Generating processing report");
        tokio::time::sleep(Duration::from_millis(200)).await;
        Ok::<_, anyhow::Error>(())
    })
    .await?;

    let output = ProcessingOutput {
        total_datasets: input.dataset_ids.len(),
        successful_datasets: successful_results.len(),
        failed_datasets: failed_count,
        results: successful_results,
        total_records,
    };

    info!("\n✅ Parallel data processing completed!");

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
        .register_workflow("parallel-processing", parallel_data_processing)
        .await;

    // Create input with mixed dataset types
    let input = DataProcessingInput {
        dataset_ids: vec![
            "dataset-fast-1".to_string(),
            "dataset-large-1".to_string(),
            "dataset-small-1".to_string(),
            "dataset-slow-1".to_string(),
            "dataset-error-1".to_string(), // This will fail
            "dataset-fast-2".to_string(),
            "dataset-large-2".to_string(),
            "dataset-small-2".to_string(),
        ],
    };

    info!("Starting parallel processing workflow...\n");
    let handle = kagzi.start_workflow("parallel-processing", input).await?;
    info!("Workflow started: {}\n", handle.run_id());

    // Start worker
    let worker = kagzi.create_worker();
    tokio::spawn(async move {
        worker.start().await.expect("Worker failed");
    });

    // Wait for result
    let result_value = handle.result().await?;
    let result: ProcessingOutput = serde_json::from_value(result_value)?;

    info!("\n=== Processing Summary ===");
    info!("Total datasets: {}", result.total_datasets);
    info!("Successful: {}", result.successful_datasets);
    info!("Failed: {}", result.failed_datasets);
    info!("Total records processed: {}", result.total_records);
    info!("\nDataset Details:");
    for dataset_result in result.results {
        info!(
            "  - {}: {} records ({}ms)",
            dataset_result.dataset_id,
            dataset_result.records_processed,
            dataset_result.processing_time_ms
        );
    }

    info!("\n💡 Key Features Demonstrated:");
    info!("  • Parallel execution with ParallelExecutor");
    info!("  • FailFast strategy in preprocessing (stops on first error)");
    info!("  • CollectAll strategy in processing (continues despite errors)");
    info!("  • Automatic memoization of parallel steps");
    info!("  • Mixed parallel and sequential execution");

    Ok(())
}
