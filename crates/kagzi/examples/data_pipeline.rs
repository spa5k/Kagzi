//! Example: Data Processing Pipeline
//!
//! This example demonstrates:
//! - Multi-step data transformation pipeline
//! - Passing data between steps with memoization
//! - Data validation and transformation
//!
//! Run with: cargo run --example data_pipeline

use kagzi::{Kagzi, WorkflowContext};
use serde::{Deserialize, Serialize};
use std::env;

#[derive(Debug, Serialize, Deserialize, Clone)]
struct RawData {
    records: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct ValidatedData {
    records: Vec<String>,
    invalid_count: usize,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct TransformedData {
    records: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct EnrichedData {
    records: Vec<Record>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct Record {
    id: String,
    data: String,
    metadata: Metadata,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct Metadata {
    processed_at: String,
    enrichment_score: u32,
}

#[derive(Debug, Serialize, Deserialize)]
struct PipelineResult {
    total_records: usize,
    successful_records: usize,
    invalid_records: usize,
}

/// Data processing pipeline workflow
async fn data_pipeline_workflow(
    ctx: WorkflowContext,
    input: RawData,
) -> anyhow::Result<PipelineResult> {
    println!(
        "📊 Starting data pipeline with {} records",
        input.records.len()
    );

    // Step 1: Validate data
    let validated: ValidatedData = ctx
        .step("validate-data", async {
            println!("  🔍 Validating data...");
            let mut valid_records = Vec::new();
            let mut invalid_count = 0;

            for record in &input.records {
                // Simple validation: check if record is not empty and has minimum length
                if record.len() >= 3 {
                    valid_records.push(record.clone());
                } else {
                    invalid_count += 1;
                }
            }

            println!(
                "     ✓ Valid: {}, Invalid: {}",
                valid_records.len(),
                invalid_count
            );

            Ok::<_, anyhow::Error>(ValidatedData {
                records: valid_records,
                invalid_count,
            })
        })
        .await?;

    // Step 2: Transform data
    let transformed: TransformedData = ctx
        .step("transform-data", async {
            println!("  🔄 Transforming {} records...", validated.records.len());

            let transformed_records: Vec<String> = validated
                .records
                .iter()
                .map(|r| format!("TRANSFORMED_{}", r.to_uppercase()))
                .collect();

            println!("     ✓ Transformed {} records", transformed_records.len());

            Ok::<_, anyhow::Error>(TransformedData {
                records: transformed_records,
            })
        })
        .await?;

    // Step 3: Enrich data
    let enriched: EnrichedData = ctx
        .step("enrich-data", async {
            println!("  ✨ Enriching {} records...", transformed.records.len());

            let enriched_records: Vec<Record> = transformed
                .records
                .iter()
                .enumerate()
                .map(|(idx, data)| Record {
                    id: format!("record_{}", idx + 1),
                    data: data.clone(),
                    metadata: Metadata {
                        processed_at: chrono::Utc::now().to_rfc3339(),
                        enrichment_score: (idx as u32 + 1) * 10,
                    },
                })
                .collect();

            println!("     ✓ Enriched {} records", enriched_records.len());

            Ok::<_, anyhow::Error>(EnrichedData {
                records: enriched_records,
            })
        })
        .await?;

    // Step 4: Save to database (simulated)
    ctx.step("save-to-database", async {
        println!(
            "  💾 Saving {} records to database...",
            enriched.records.len()
        );

        // Simulate database save
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;

        println!("     ✓ Saved successfully");

        Ok::<_, anyhow::Error>(())
    })
    .await?;

    let result = PipelineResult {
        total_records: input.records.len(),
        successful_records: enriched.records.len(),
        invalid_records: validated.invalid_count,
    };

    println!(
        "✅ Pipeline completed! Processed {}/{} records ({} invalid)",
        result.successful_records, result.total_records, result.invalid_records
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
        .register_workflow("data-pipeline", data_pipeline_workflow)
        .await;

    // Sample data with some invalid records
    let sample_data = RawData {
        records: vec![
            "user_data_001".to_string(),
            "user_data_002".to_string(),
            "ab".to_string(), // Too short - invalid
            "user_data_003".to_string(),
            "".to_string(), // Empty - invalid
            "user_data_004".to_string(),
            "user_data_005".to_string(),
        ],
    };

    println!("Starting data pipeline workflow...");
    let handle = kagzi.start_workflow("data-pipeline", sample_data).await?;

    println!("Workflow started: {}", handle.run_id());

    // Start worker in background
    let worker = kagzi.create_worker();
    tokio::spawn(async move {
        worker.start().await.expect("Worker failed");
    });

    // Wait for result
    println!("Waiting for result...");
    let result = handle.result().await?;
    println!("\nFinal Result: {}", serde_json::to_string_pretty(&result)?);

    Ok(())
}
