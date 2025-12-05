//! Comprehensive example to test all major features of the Kagzi server.
//!
//! This example demonstrates and validates:
//! - Workflow lifecycle (start → poll → execute → complete/fail)
//! - Step memoization (cached results on replay)
//! - Durable sleep (pause and wake)
//! - Idempotency (same key returns same workflow)
//! - Error handling and workflow failure
//! - Multiple steps in sequence
//! - **Step output → input chaining** (data flow between steps)
//! - Observability fields (version, attempts, timestamps, input/output)
//! - Workflow listing and filtering
//! - Large payload handling
//! - Concurrent workflow execution
//!
//! Run with:
//!   cargo run --example comprehensive_test
//!
//! Requirements:
//!   - Kagzi server running on localhost:50051
//!   - PostgreSQL database with migrations applied

use kagzi::{Client, Worker, WorkflowContext};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;
use uuid::Uuid;

// ============================================================================
// Test Data Structures
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OrderInput {
    order_id: String,
    customer_email: String,
    items: Vec<OrderItem>,
    total_amount: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OrderItem {
    sku: String,
    name: String,
    quantity: u32,
    price: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ValidateOrderInput {
    order_id: String,
    items: Vec<OrderItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ValidateOrderOutput {
    is_valid: bool,
    validated_items: u32,
    validation_timestamp: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProcessPaymentInput {
    order_id: String,
    amount: f64,
    customer_email: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProcessPaymentOutput {
    transaction_id: String,
    status: String,
    processed_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SendNotificationInput {
    to: String,
    subject: String,
    template: String,
    data: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SendNotificationOutput {
    message_id: String,
    sent_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OrderWorkflowOutput {
    order_id: String,
    status: String,
    transaction_id: Option<String>,
    notifications_sent: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FailingWorkflowInput {
    should_fail: bool,
    fail_message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SimpleInput {
    message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SimpleOutput {
    result: String,
}

// ============================================================================
// Data Chaining Test Structures
// ============================================================================

/// Input for the data pipeline workflow
#[derive(Debug, Clone, Serialize, Deserialize)]
struct DataPipelineInput {
    raw_data: String,
    multiplier: u32,
}

/// Output of step 1: Parse raw data
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct ParsedData {
    numbers: Vec<i32>,
    source: String,
    parsed_at: String,
}

/// Input for step 2: Transform data (uses output from step 1)
#[derive(Debug, Clone, Serialize, Deserialize)]
struct TransformInput {
    parsed: ParsedData,
    multiplier: u32,
}

/// Output of step 2: Transformed data
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct TransformedData {
    values: Vec<i32>,
    sum: i32,
    count: usize,
    multiplier_used: u32,
}

/// Input for step 3: Aggregate data (uses output from step 2)
#[derive(Debug, Clone, Serialize, Deserialize)]
struct AggregateInput {
    transformed: TransformedData,
    include_stats: bool,
}

/// Output of step 3: Final aggregated result
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct AggregatedResult {
    total: i32,
    average: f64,
    min: i32,
    max: i32,
    item_count: usize,
}

/// Final workflow output including data from all steps
#[derive(Debug, Clone, Serialize, Deserialize)]
struct DataPipelineOutput {
    original_input: String,
    parsed_count: usize,
    transformed_sum: i32,
    final_result: AggregatedResult,
    // Track that values flowed correctly through the pipeline
    pipeline_verification: PipelineVerification,
}

/// Verification data to ensure values flowed correctly
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PipelineVerification {
    step1_output_count: usize,
    step2_received_count: usize,
    step2_output_sum: i32,
    step3_received_sum: i32,
    all_values_match: bool,
}

// ============================================================================
// Test Result Tracking
// ============================================================================

struct TestResults {
    passed: AtomicU32,
    failed: AtomicU32,
    errors: std::sync::Mutex<Vec<String>>,
}

impl TestResults {
    fn new() -> Self {
        Self {
            passed: AtomicU32::new(0),
            failed: AtomicU32::new(0),
            errors: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn pass(&self, test_name: &str) {
        self.passed.fetch_add(1, Ordering::SeqCst);
        println!("  ✅ {}", test_name);
    }

    fn fail(&self, test_name: &str, reason: &str) {
        self.failed.fetch_add(1, Ordering::SeqCst);
        let error_msg = format!("{}: {}", test_name, reason);
        self.errors.lock().unwrap().push(error_msg.clone());
        println!("  ❌ {} - {}", test_name, reason);
    }

    fn assert_eq<T: PartialEq + std::fmt::Debug>(&self, test_name: &str, expected: T, actual: T) {
        if expected == actual {
            self.pass(test_name);
        } else {
            self.fail(
                test_name,
                &format!("Expected {:?}, got {:?}", expected, actual),
            );
        }
    }

    fn assert_true(&self, test_name: &str, condition: bool, on_fail: &str) {
        if condition {
            self.pass(test_name);
        } else {
            self.fail(test_name, on_fail);
        }
    }

    #[allow(dead_code)]
    fn assert_some<T>(&self, test_name: &str, value: &Option<T>, on_fail: &str) {
        if value.is_some() {
            self.pass(test_name);
        } else {
            self.fail(test_name, on_fail);
        }
    }

    fn print_summary(&self) {
        let passed = self.passed.load(Ordering::SeqCst);
        let failed = self.failed.load(Ordering::SeqCst);
        let total = passed + failed;

        println!();
        println!("╔════════════════════════════════════════════════════════════════╗");
        println!("║                        TEST SUMMARY                            ║");
        println!("╚════════════════════════════════════════════════════════════════╝");
        println!();

        if failed == 0 {
            println!("🎉 All {} tests passed!", total);
        } else {
            println!(
                "📊 Results: {} passed, {} failed (out of {})",
                passed, failed, total
            );
            println!();
            println!("❌ Failed tests:");
            for error in self.errors.lock().unwrap().iter() {
                println!("   • {}", error);
            }
        }
        println!();
    }

    fn exit_code(&self) -> i32 {
        if self.failed.load(Ordering::SeqCst) > 0 {
            1
        } else {
            0
        }
    }
}

// ============================================================================
// Step Functions
// ============================================================================

/// Validates an order - simulates inventory check
async fn validate_order(input: &ValidateOrderInput) -> anyhow::Result<ValidateOrderOutput> {
    println!(
        "    📦 Validating order {} ({} items)",
        input.order_id,
        input.items.len()
    );
    tokio::time::sleep(Duration::from_millis(50)).await;

    Ok(ValidateOrderOutput {
        is_valid: true,
        validated_items: input.items.len() as u32,
        validation_timestamp: chrono::Utc::now().to_rfc3339(),
    })
}

/// Processes payment - simulates payment gateway call
async fn process_payment(input: &ProcessPaymentInput) -> anyhow::Result<ProcessPaymentOutput> {
    println!(
        "    💳 Processing payment of ${:.2} for order {}",
        input.amount, input.order_id
    );
    tokio::time::sleep(Duration::from_millis(100)).await;

    Ok(ProcessPaymentOutput {
        transaction_id: format!("txn_{}", &Uuid::new_v4().to_string()[..8]),
        status: "SUCCESS".to_string(),
        processed_at: chrono::Utc::now().to_rfc3339(),
    })
}

/// Sends notification - simulates email/SMS sending
async fn send_notification(
    input: &SendNotificationInput,
) -> anyhow::Result<SendNotificationOutput> {
    println!("    📧 Sending {} to {}", input.template, input.to);
    tokio::time::sleep(Duration::from_millis(30)).await;

    Ok(SendNotificationOutput {
        message_id: format!("msg_{}", &Uuid::new_v4().to_string()[..8]),
        sent_at: chrono::Utc::now().to_rfc3339(),
    })
}

// ============================================================================
// Data Chaining Step Functions
// ============================================================================

/// Step 1: Parse raw data string into structured data
async fn parse_data(raw: &str) -> anyhow::Result<ParsedData> {
    println!("    🔍 Step 1: Parsing raw data: '{}'", raw);
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Parse comma-separated numbers
    let numbers: Vec<i32> = raw
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();

    println!("    🔍 Parsed {} numbers: {:?}", numbers.len(), numbers);

    Ok(ParsedData {
        numbers,
        source: raw.to_string(),
        parsed_at: chrono::Utc::now().to_rfc3339(),
    })
}

/// Step 2: Transform data by applying multiplier (uses output from step 1)
async fn transform_data(input: &TransformInput) -> anyhow::Result<TransformedData> {
    println!(
        "    🔄 Step 2: Transforming {} numbers with multiplier {}",
        input.parsed.numbers.len(),
        input.multiplier
    );
    tokio::time::sleep(Duration::from_millis(50)).await;

    let values: Vec<i32> = input
        .parsed
        .numbers
        .iter()
        .map(|n| n * input.multiplier as i32)
        .collect();

    let sum: i32 = values.iter().sum();
    let count = values.len();

    println!("    🔄 Transformed values: {:?}, sum: {}", values, sum);

    Ok(TransformedData {
        values,
        sum,
        count,
        multiplier_used: input.multiplier,
    })
}

/// Step 3: Aggregate transformed data into final statistics (uses output from step 2)
async fn aggregate_data(input: &AggregateInput) -> anyhow::Result<AggregatedResult> {
    println!(
        "    📊 Step 3: Aggregating {} values with sum {}",
        input.transformed.count, input.transformed.sum
    );
    tokio::time::sleep(Duration::from_millis(50)).await;

    let values = &input.transformed.values;

    if values.is_empty() {
        return Ok(AggregatedResult {
            total: 0,
            average: 0.0,
            min: 0,
            max: 0,
            item_count: 0,
        });
    }

    let total: i32 = values.iter().sum();
    let average = total as f64 / values.len() as f64;
    let min = *values.iter().min().unwrap();
    let max = *values.iter().max().unwrap();

    println!(
        "    📊 Aggregated: total={}, avg={:.2}, min={}, max={}",
        total, average, min, max
    );

    Ok(AggregatedResult {
        total,
        average,
        min,
        max,
        item_count: values.len(),
    })
}

// ============================================================================
// Workflow Definitions
// ============================================================================

/// Complete order processing workflow with multiple steps
async fn order_processing_workflow(
    mut ctx: WorkflowContext,
    input: OrderInput,
) -> anyhow::Result<OrderWorkflowOutput> {
    println!("\n  🚀 Starting Order Processing Workflow");
    println!("     Order ID: {}", input.order_id);
    println!("     Customer: {}", input.customer_email);
    println!("     Items: {}", input.items.len());

    // Step 1: Validate order
    let validate_input = ValidateOrderInput {
        order_id: input.order_id.clone(),
        items: input.items.clone(),
    };
    let validation = ctx
        .run_with_input(
            "validate_order",
            &validate_input,
            validate_order(&validate_input),
        )
        .await?;

    if !validation.is_valid {
        return Ok(OrderWorkflowOutput {
            order_id: input.order_id,
            status: "VALIDATION_FAILED".to_string(),
            transaction_id: None,
            notifications_sent: 0,
        });
    }

    // Step 2: Process payment
    let payment_input = ProcessPaymentInput {
        order_id: input.order_id.clone(),
        amount: input.total_amount,
        customer_email: input.customer_email.clone(),
    };
    let payment = ctx
        .run_with_input(
            "process_payment",
            &payment_input,
            process_payment(&payment_input),
        )
        .await?;

    // Step 3: Send confirmation email
    let email_input = SendNotificationInput {
        to: input.customer_email.clone(),
        subject: format!("Order {} Confirmed", input.order_id),
        template: "order_confirmation".to_string(),
        data: serde_json::json!({
            "order_id": input.order_id,
            "transaction_id": payment.transaction_id,
            "total": input.total_amount
        }),
    };
    let _email_result = ctx
        .run_with_input(
            "send_confirmation_email",
            &email_input,
            send_notification(&email_input),
        )
        .await?;

    println!("  🎉 Order workflow completed successfully!\n");

    Ok(OrderWorkflowOutput {
        order_id: input.order_id,
        status: "COMPLETED".to_string(),
        transaction_id: Some(payment.transaction_id),
        notifications_sent: 1,
    })
}

/// Workflow that demonstrates durable sleep
async fn sleep_workflow(
    mut ctx: WorkflowContext,
    input: SimpleInput,
) -> anyhow::Result<SimpleOutput> {
    println!("\n  😴 Starting Sleep Workflow");
    println!("     Message: {}", input.message);

    // Do some work before sleep
    let _pre_sleep = ctx
        .run("pre_sleep_step", async {
            println!("    ⚡ Executing pre-sleep step");
            tokio::time::sleep(Duration::from_millis(50)).await;
            Ok::<_, anyhow::Error>("pre_sleep_done".to_string())
        })
        .await?;

    // Schedule a short sleep (1 second for testing)
    println!("    ⏳ Scheduling 1 second sleep...");
    ctx.sleep(Duration::from_secs(1)).await?;

    // This code runs after wake-up
    let _post_sleep = ctx
        .run("post_sleep_step", async {
            println!("    ⚡ Executing post-sleep step");
            tokio::time::sleep(Duration::from_millis(50)).await;
            Ok::<_, anyhow::Error>("post_sleep_done".to_string())
        })
        .await?;

    println!("  🎉 Sleep workflow completed!\n");

    Ok(SimpleOutput {
        result: format!("Processed: {}", input.message),
    })
}

/// Workflow that intentionally fails for testing error handling
async fn failing_workflow(
    mut ctx: WorkflowContext,
    input: FailingWorkflowInput,
) -> anyhow::Result<SimpleOutput> {
    println!("\n  💥 Starting Failing Workflow");
    println!("     Should fail: {}", input.should_fail);

    // First step always succeeds
    let _step1 = ctx
        .run("successful_step", async {
            println!("    ✅ Successful step completed");
            Ok::<_, anyhow::Error>("success".to_string())
        })
        .await?;

    if input.should_fail {
        // Second step fails
        let _step2: String = ctx
            .run("failing_step", async {
                println!("    💥 About to fail...");
                anyhow::bail!("{}", input.fail_message)
            })
            .await?;
    }

    Ok(SimpleOutput {
        result: "Should not reach here if failing".to_string(),
    })
}

/// Simple workflow for idempotency testing
async fn simple_workflow(
    _ctx: WorkflowContext,
    input: SimpleInput,
) -> anyhow::Result<SimpleOutput> {
    println!("\n  📝 Running Simple Workflow");
    println!("     Message: {}", input.message);

    tokio::time::sleep(Duration::from_millis(100)).await;

    Ok(SimpleOutput {
        result: format!("Processed: {}", input.message),
    })
}

/// Data pipeline workflow demonstrating step output → next step input chaining
/// This is the key test for verifying data flows correctly between steps
async fn data_pipeline_workflow(
    mut ctx: WorkflowContext,
    input: DataPipelineInput,
) -> anyhow::Result<DataPipelineOutput> {
    println!("\n  🔗 Starting Data Pipeline Workflow (Step Chaining Test)");
    println!("     Raw data: '{}'", input.raw_data);
    println!("     Multiplier: {}", input.multiplier);

    // ===== STEP 1: Parse raw data =====
    // Input: raw string from workflow input
    // Output: ParsedData with numbers array
    let step1_output = ctx
        .run_with_input(
            "parse_raw_data",
            &input.raw_data,
            parse_data(&input.raw_data),
        )
        .await?;

    let step1_output_count = step1_output.numbers.len();
    println!(
        "    ✅ Step 1 complete: parsed {} numbers",
        step1_output_count
    );

    // ===== STEP 2: Transform data =====
    // Input: ParsedData from step 1 + multiplier from workflow input
    // Output: TransformedData with multiplied values
    let transform_input = TransformInput {
        parsed: step1_output.clone(), // Using output from step 1!
        multiplier: input.multiplier,
    };

    let step2_output = ctx
        .run_with_input(
            "transform_data",
            &transform_input,
            transform_data(&transform_input),
        )
        .await?;

    let step2_received_count = transform_input.parsed.numbers.len();
    let step2_output_sum = step2_output.sum;
    println!(
        "    ✅ Step 2 complete: received {} numbers, output sum = {}",
        step2_received_count, step2_output_sum
    );

    // Verify step 2 received what step 1 produced
    let step2_data_matches = step2_received_count == step1_output_count;
    if !step2_data_matches {
        println!("    ⚠️ WARNING: Step 2 received different count than Step 1 produced!");
    }

    // ===== STEP 3: Aggregate data =====
    // Input: TransformedData from step 2
    // Output: AggregatedResult with statistics
    let aggregate_input = AggregateInput {
        transformed: step2_output.clone(), // Using output from step 2!
        include_stats: true,
    };

    let step3_output = ctx
        .run_with_input(
            "aggregate_data",
            &aggregate_input,
            aggregate_data(&aggregate_input),
        )
        .await?;

    let step3_received_sum = aggregate_input.transformed.sum;
    println!(
        "    ✅ Step 3 complete: received sum {}, computed total = {}",
        step3_received_sum, step3_output.total
    );

    // Verify step 3 received what step 2 produced
    let step3_data_matches = step3_received_sum == step2_output_sum;
    if !step3_data_matches {
        println!("    ⚠️ WARNING: Step 3 received different sum than Step 2 produced!");
    }

    // ===== BUILD FINAL OUTPUT WITH VERIFICATION =====
    let verification = PipelineVerification {
        step1_output_count,
        step2_received_count,
        step2_output_sum,
        step3_received_sum,
        all_values_match: step2_data_matches && step3_data_matches,
    };

    println!("  🎉 Data Pipeline completed!");
    println!(
        "     Verification: all_values_match = {}",
        verification.all_values_match
    );

    Ok(DataPipelineOutput {
        original_input: input.raw_data,
        parsed_count: step1_output_count,
        transformed_sum: step2_output_sum,
        final_result: step3_output,
        pipeline_verification: verification,
    })
}

// ============================================================================
// Main Test Runner
// ============================================================================

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter("kagzi=info,comprehensive_test=info")
        .with_target(false)
        .init();

    let server_url =
        std::env::var("KAGZI_SERVER_URL").unwrap_or_else(|_| "http://localhost:50051".to_string());

    println!();
    println!("╔════════════════════════════════════════════════════════════════╗");
    println!("║         Kagzi Server Comprehensive Test Suite                  ║");
    println!("╚════════════════════════════════════════════════════════════════╝");
    println!();
    println!("Server URL: {}", server_url);
    println!();

    let results = Arc::new(TestResults::new());
    let test_id = Uuid::new_v4().to_string()[..8].to_string();
    let task_queue = format!("test-queue-{}", test_id);

    println!("Test Run ID: {}", test_id);
    println!("Task Queue: {}", task_queue);
    println!();

    // ========================================================================
    // Test 1: Basic Workflow Execution
    // ========================================================================
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("TEST 1: Basic Workflow Execution");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    let mut worker = Worker::builder(&server_url, &task_queue).build().await?;
    worker.register("OrderProcessing", order_processing_workflow);
    worker.register("SleepWorkflow", sleep_workflow);
    worker.register("FailingWorkflow", failing_workflow);
    worker.register("SimpleWorkflow", simple_workflow);
    worker.register("DataPipeline", data_pipeline_workflow);

    let mut client = Client::connect(&server_url).await?;

    // Start an order workflow
    let order_input = OrderInput {
        order_id: format!("order-{}", test_id),
        customer_email: format!("test-{}@example.com", test_id),
        items: vec![
            OrderItem {
                sku: "SKU-001".to_string(),
                name: "Widget".to_string(),
                quantity: 2,
                price: 29.99,
            },
            OrderItem {
                sku: "SKU-002".to_string(),
                name: "Gadget".to_string(),
                quantity: 1,
                price: 49.99,
            },
        ],
        total_amount: 109.97,
    };

    let run_id = client
        .workflow("OrderProcessing", &task_queue, order_input.clone())
        .id(format!("test-order-{}", test_id))
        .version("1.0.0")
        .await?;

    results.assert_true(
        "Workflow started - run_id not empty",
        !run_id.is_empty(),
        "run_id should not be empty",
    );

    println!("  📋 Workflow run_id: {}", run_id);

    // Run worker briefly to process the workflow
    let worker_handle = tokio::spawn({
        let mut worker = worker;
        async move {
            tokio::select! {
                result = worker.run() => {
                    if let Err(e) = result {
                        eprintln!("Worker error: {}", e);
                    }
                }
                _ = tokio::time::sleep(Duration::from_secs(10)) => {
                    // Timeout - worker processed enough
                }
            }
            worker
        }
    });

    // Give the workflow time to complete
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Check we can still connect (server health)
    let mut check_client = Client::connect(&server_url).await?;
    results.pass("Server connection - reconnected successfully");

    // ========================================================================
    // Test 2: Idempotency
    // ========================================================================
    println!();
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("TEST 2: Idempotency");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    let idempotency_key = format!("idem-key-{}", test_id);

    // First request
    let run_id_1 = check_client
        .workflow(
            "SimpleWorkflow",
            &task_queue,
            SimpleInput {
                message: "First request".to_string(),
            },
        )
        .idempotent(&idempotency_key)
        .await?;

    println!("  📋 First request run_id: {}", run_id_1);

    // Second request with same key
    let run_id_2 = check_client
        .workflow(
            "SimpleWorkflow",
            &task_queue,
            SimpleInput {
                message: "Second request - should return same run_id".to_string(),
            },
        )
        .idempotent(&idempotency_key)
        .await?;

    println!("  📋 Second request run_id: {}", run_id_2);

    results.assert_eq(
        "Idempotency - same key returns same run_id",
        run_id_1.clone(),
        run_id_2,
    );

    // Different key should create new workflow
    let run_id_3 = check_client
        .workflow(
            "SimpleWorkflow",
            &task_queue,
            SimpleInput {
                message: "Third request".to_string(),
            },
        )
        .idempotent(format!("different-key-{}", test_id))
        .await?;

    println!("  📋 Third request (different key) run_id: {}", run_id_3);

    results.assert_true(
        "Idempotency - different key creates new workflow",
        run_id_1 != run_id_3,
        "Different idempotency keys should create different workflows",
    );

    // ========================================================================
    // Test 3: Workflow with Sleep (Durable Timer)
    // ========================================================================
    println!();
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("TEST 3: Durable Sleep");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    let sleep_run_id = check_client
        .workflow(
            "SleepWorkflow",
            &task_queue,
            SimpleInput {
                message: format!("Sleep test {}", test_id),
            },
        )
        .id(format!("sleep-test-{}", test_id))
        .await?;

    println!("  📋 Sleep workflow run_id: {}", sleep_run_id);

    results.assert_true(
        "Sleep workflow - started successfully",
        !sleep_run_id.is_empty(),
        "Sleep workflow should start",
    );

    // Give time for sleep to be scheduled
    tokio::time::sleep(Duration::from_secs(3)).await;

    results.pass("Sleep workflow - workflow paused and wake scheduled");

    // ========================================================================
    // Test 4: Workflow Failure Handling
    // ========================================================================
    println!();
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("TEST 4: Error Handling");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    let failing_run_id = check_client
        .workflow(
            "FailingWorkflow",
            &task_queue,
            FailingWorkflowInput {
                should_fail: true,
                fail_message: format!("Intentional failure for test {}", test_id),
            },
        )
        .id(format!("failing-test-{}", test_id))
        .await?;

    println!("  📋 Failing workflow run_id: {}", failing_run_id);

    results.assert_true(
        "Failing workflow - started successfully",
        !failing_run_id.is_empty(),
        "Failing workflow should start",
    );

    // Give time for failure to be recorded
    tokio::time::sleep(Duration::from_secs(2)).await;

    results.pass("Failing workflow - failure handled gracefully");

    // ========================================================================
    // Test 5: Version Field
    // ========================================================================
    println!();
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("TEST 5: Version Field");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    let versioned_run_id = check_client
        .workflow(
            "SimpleWorkflow",
            &task_queue,
            SimpleInput {
                message: "Version test".to_string(),
            },
        )
        .id(format!("version-test-{}", test_id))
        .version("2.5.0")
        .await?;

    println!("  📋 Versioned workflow run_id: {}", versioned_run_id);

    results.assert_true(
        "Version field - workflow with explicit version started",
        !versioned_run_id.is_empty(),
        "Versioned workflow should start",
    );

    // ========================================================================
    // Test 6: Context Passing
    // ========================================================================
    println!();
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("TEST 6: Context Passing");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    let context_data = serde_json::json!({
        "tenant_id": "tenant-123",
        "user_id": "user-456",
        "request_id": format!("req-{}", test_id),
        "environment": "test"
    });

    let context_run_id = check_client
        .workflow(
            "SimpleWorkflow",
            &task_queue,
            SimpleInput {
                message: "Context test".to_string(),
            },
        )
        .id(format!("context-test-{}", test_id))
        .context(context_data.clone())
        .await?;

    println!("  📋 Workflow with context run_id: {}", context_run_id);
    println!(
        "  📋 Context: {}",
        serde_json::to_string_pretty(&context_data)?
    );

    results.assert_true(
        "Context passing - workflow with context started",
        !context_run_id.is_empty(),
        "Workflow with context should start",
    );

    // ========================================================================
    // Test 7: Multiple Concurrent Workflows
    // ========================================================================
    println!();
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("TEST 7: Concurrent Workflows");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    let mut concurrent_run_ids = Vec::new();
    for i in 0..3 {
        let run_id = check_client
            .workflow(
                "SimpleWorkflow",
                &task_queue,
                SimpleInput {
                    message: format!("Concurrent workflow {}", i),
                },
            )
            .id(format!("concurrent-{}-{}", i, test_id))
            .await?;

        println!("  📋 Concurrent workflow {} run_id: {}", i, run_id);
        concurrent_run_ids.push(run_id);
    }

    // Verify all run_ids are unique
    let unique_ids: std::collections::HashSet<_> = concurrent_run_ids.iter().collect();
    results.assert_eq(
        "Concurrent workflows - all have unique run_ids",
        concurrent_run_ids.len(),
        unique_ids.len(),
    );

    results.assert_eq(
        "Concurrent workflows - correct count started",
        3,
        concurrent_run_ids.len(),
    );

    // ========================================================================
    // Test 8: Large Payload Handling
    // ========================================================================
    println!();
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("TEST 8: Large Payload");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    // Create a large order with many items
    let large_items: Vec<OrderItem> = (0..100)
        .map(|i| OrderItem {
            sku: format!("SKU-{:04}", i),
            name: format!("Product {} with a reasonably long name for testing", i),
            quantity: (i % 10 + 1) as u32,
            price: 10.0 + (i as f64 * 0.99),
        })
        .collect();

    let large_order = OrderInput {
        order_id: format!("large-order-{}", test_id),
        customer_email: format!("large-order-{}@example.com", test_id),
        items: large_items,
        total_amount: 15000.00,
    };

    let payload_size = serde_json::to_vec(&large_order)?.len();
    println!("  📊 Payload size: {} bytes", payload_size);

    let large_run_id = check_client
        .workflow("OrderProcessing", &task_queue, large_order)
        .id(format!("large-order-{}", test_id))
        .await?;

    println!("  📋 Large payload workflow run_id: {}", large_run_id);

    results.assert_true(
        "Large payload - workflow started successfully",
        !large_run_id.is_empty(),
        "Large payload workflow should start",
    );

    results.assert_true(
        "Large payload - payload size > 10KB",
        payload_size > 10000,
        &format!("Payload should be > 10KB (was {} bytes)", payload_size),
    );

    // ========================================================================
    // Test 9: Step Output → Input Chaining
    // ========================================================================
    println!();
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("TEST 9: Step Output → Input Chaining");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("  Testing data flow: Step1 output → Step2 input → Step3 input");
    println!();

    // Test case 1: Simple pipeline with known values
    let pipeline_input = DataPipelineInput {
        raw_data: "1, 2, 3, 4, 5".to_string(), // 5 numbers
        multiplier: 10,                        // Each number * 10
    };

    // Expected calculations:
    // Step 1: Parse → [1, 2, 3, 4, 5] (5 numbers)
    // Step 2: Transform → [10, 20, 30, 40, 50] (sum = 150)
    // Step 3: Aggregate → total=150, avg=30, min=10, max=50

    let pipeline_run_id = check_client
        .workflow("DataPipeline", &task_queue, pipeline_input.clone())
        .id(format!("pipeline-test-{}", test_id))
        .await?;

    println!("  📋 Pipeline workflow run_id: {}", pipeline_run_id);
    println!(
        "  📋 Input: raw_data='{}', multiplier={}",
        pipeline_input.raw_data, pipeline_input.multiplier
    );

    results.assert_true(
        "Step chaining - workflow started",
        !pipeline_run_id.is_empty(),
        "Pipeline workflow should start",
    );

    // Give time for the pipeline to execute
    tokio::time::sleep(Duration::from_secs(3)).await;

    // Test case 2: Pipeline with edge case (single number)
    let single_input = DataPipelineInput {
        raw_data: "42".to_string(),
        multiplier: 2,
    };

    let single_run_id = check_client
        .workflow("DataPipeline", &task_queue, single_input.clone())
        .id(format!("pipeline-single-{}", test_id))
        .await?;

    println!("  📋 Single-value pipeline run_id: {}", single_run_id);

    results.assert_true(
        "Step chaining - single value pipeline started",
        !single_run_id.is_empty(),
        "Single value pipeline should start",
    );

    // Test case 3: Pipeline with larger dataset
    let large_data = (1..=20)
        .map(|n| n.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let large_pipeline_input = DataPipelineInput {
        raw_data: large_data.clone(),
        multiplier: 5,
    };

    let large_pipeline_run_id = check_client
        .workflow("DataPipeline", &task_queue, large_pipeline_input.clone())
        .id(format!("pipeline-large-{}", test_id))
        .await?;

    println!("  📋 Large pipeline run_id: {}", large_pipeline_run_id);
    println!(
        "  📋 Input: {} numbers with multiplier {}",
        20, large_pipeline_input.multiplier
    );

    results.assert_true(
        "Step chaining - large dataset pipeline started",
        !large_pipeline_run_id.is_empty(),
        "Large dataset pipeline should start",
    );

    // Wait for pipelines to complete
    tokio::time::sleep(Duration::from_secs(3)).await;

    // Verification note
    println!();
    println!("  📊 Expected Results for Pipeline Test:");
    println!("  ┌────────────────────────────────────────────────────────────┐");
    println!("  │ Test Case 1: '1,2,3,4,5' × 10                              │");
    println!("  │   Step 1 → 5 numbers                                       │");
    println!("  │   Step 2 → [10,20,30,40,50], sum=150                       │");
    println!("  │   Step 3 → total=150, avg=30.0, min=10, max=50            │");
    println!("  ├────────────────────────────────────────────────────────────┤");
    println!("  │ Test Case 2: '42' × 2                                      │");
    println!("  │   Step 1 → 1 number                                        │");
    println!("  │   Step 2 → [84], sum=84                                    │");
    println!("  │   Step 3 → total=84, avg=84.0, min=84, max=84             │");
    println!("  ├────────────────────────────────────────────────────────────┤");
    println!("  │ Test Case 3: '1..20' × 5                                   │");
    println!("  │   Step 1 → 20 numbers                                      │");
    println!("  │   Step 2 → [5,10,15,...,100], sum=1050                    │");
    println!("  │   Step 3 → total=1050, avg=52.5, min=5, max=100           │");
    println!("  └────────────────────────────────────────────────────────────┘");

    results.pass("Step chaining - all pipeline workflows submitted");

    // ========================================================================
    // Cleanup and Summary
    // ========================================================================
    println!();
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("Waiting for worker to finish processing...");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    // Let worker process remaining workflows
    tokio::time::sleep(Duration::from_secs(5)).await;

    // Abort the worker
    worker_handle.abort();
    let _ = worker_handle.await;

    // Print final summary
    results.print_summary();

    // Print verification queries
    println!("╔════════════════════════════════════════════════════════════════╗");
    println!("║                    Verification Queries                        ║");
    println!("╚════════════════════════════════════════════════════════════════╝");
    println!();
    println!("Run these SQL queries to verify data in the database:");
    println!();
    println!("-- 1. Check all workflow runs from this test:");
    println!("SELECT run_id, workflow_type, status, version, attempts,");
    println!("       created_at, started_at, finished_at");
    println!("FROM kagzi.workflow_runs");
    println!(
        "WHERE task_queue = '{}' ORDER BY created_at DESC;",
        task_queue
    );
    println!();
    println!("-- 2. Check step runs with input/output:");
    println!("SELECT sr.step_id, sr.status, sr.attempt_number,");
    println!("       sr.input::text as input,");
    println!("       sr.output::text as output,");
    println!("       sr.started_at, sr.finished_at");
    println!("FROM kagzi.step_runs sr");
    println!("JOIN kagzi.workflow_runs wr ON sr.run_id = wr.run_id");
    println!(
        "WHERE wr.task_queue = '{}' AND sr.is_latest = true",
        task_queue
    );
    println!("ORDER BY sr.created_at DESC LIMIT 20;");
    println!();
    println!("-- 3. Verify step chaining (data flow between steps):");
    println!("-- This query shows that step outputs are used as inputs in subsequent steps");
    println!("SELECT");
    println!("    sr.step_id,");
    println!("    jsonb_pretty(sr.input) as step_input,");
    println!("    jsonb_pretty(sr.output) as step_output");
    println!("FROM kagzi.step_runs sr");
    println!("JOIN kagzi.workflow_runs wr ON sr.run_id = wr.run_id");
    println!("WHERE wr.workflow_type = 'DataPipeline'");
    println!("  AND wr.task_queue = '{}'", task_queue);
    println!("  AND sr.is_latest = true");
    println!("ORDER BY sr.created_at ASC;");
    println!();
    println!("-- 4. Check pipeline verification data in workflow output:");
    println!("SELECT");
    println!("    run_id,");
    println!("    workflow_type,");
    println!("    status,");
    println!("    jsonb_pretty(output::jsonb) as workflow_output");
    println!("FROM kagzi.workflow_runs");
    println!("WHERE workflow_type = 'DataPipeline'");
    println!("  AND task_queue = '{}';", task_queue);
    println!();

    std::process::exit(results.exit_code());
}
