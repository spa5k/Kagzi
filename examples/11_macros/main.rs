//! Example demonstrating the new Kagzi macros.
//!
//! This example shows how the simplified macros make workflow definitions
//! more readable and observable.

use std::time::Duration;

use kagzi::WorkflowContext;
use kagzi_macros::kagzi_step;
use serde::{Deserialize, Serialize};
use tracing_subscriber::EnvFilter;

// Define input and output types for our workflows
#[derive(Debug, Serialize, Deserialize, Clone)]
struct ProcessOrderInput {
    order_id: String,
    items: Vec<OrderItem>,
    customer_id: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct OrderItem {
    product_id: String,
    quantity: u32,
    price: f64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[allow(dead_code)]
struct ProcessOrderOutput {
    order_id: String,
    total_amount: f64,
    status: String,
    processed_items: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[allow(dead_code)]
struct OrderWithTotal {
    order: ProcessOrderInput,
    total: f64,
}

// Step function with #[kagzi_step] attribute
#[kagzi_step]
/// Validates the order input and calculates basic checks
async fn validate_order(order: ProcessOrderInput) -> anyhow::Result<ProcessOrderInput> {
    // Simulate validation
    if order.order_id.is_empty() {
        anyhow::bail!("Order ID cannot be empty");
    }

    if order.customer_id.is_empty() {
        anyhow::bail!("Customer ID cannot be empty");
    }

    if order.items.is_empty() {
        anyhow::bail!("Order must contain at least one item");
    }

    tracing::info!(
        order_id = %order.order_id,
        items_count = %order.items.len(),
        "Order validation passed"
    );

    Ok(order)
}

// Another step function with the macro
#[kagzi_step]
/// Calculates the total amount for the order
async fn calculate_total(order: ProcessOrderInput) -> anyhow::Result<OrderWithTotal> {
    let total: f64 = order
        .items
        .iter()
        .map(|item| item.price * item.quantity as f64)
        .sum();

    // Simulate processing time
    tokio::time::sleep(Duration::from_millis(100)).await;

    tracing::info!(
        order_id = %order.order_id,
        total = %total,
        "Order total calculated"
    );

    Ok(OrderWithTotal { order, total })
}

// Third step function
#[kagzi_step]
/// Processes the payment for the order
async fn process_payment(order_total: OrderWithTotal) -> anyhow::Result<(OrderWithTotal, String)> {
    // Simulate payment processing
    tokio::time::sleep(Duration::from_millis(500)).await;

    let payment_id = format!(
        "pay-{}-{}",
        order_total.order.order_id,
        uuid::Uuid::new_v4()
    );

    tracing::info!(
        order_id = %order_total.order.order_id,
        payment_id = %payment_id,
        amount = %order_total.total,
        "Payment processed successfully"
    );

    Ok((order_total, payment_id))
}

// Fourth step function
#[kagzi_step]
/// Finalizes the order and generates output
async fn finalize_order(
    (order_total, _payment_id): (OrderWithTotal, String),
) -> anyhow::Result<ProcessOrderOutput> {
    // Simulate finalization
    tokio::time::sleep(Duration::from_millis(200)).await;

    let processed_items = order_total
        .order
        .items
        .iter()
        .map(|item| format!("{} (x{})", item.product_id, item.quantity))
        .collect();

    let output = ProcessOrderOutput {
        order_id: order_total.order.order_id.clone(),
        total_amount: order_total.total,
        status: "completed".to_string(),
        processed_items,
    };

    tracing::info!(
        order_id = %output.order_id,
        total = %output.total_amount,
        status = %output.status,
        "Order finalized"
    );

    Ok(output)
}

// The workflow that uses these steps
#[allow(dead_code)]
async fn process_order_workflow(
    mut ctx: WorkflowContext,
    input: ProcessOrderInput,
) -> anyhow::Result<ProcessOrderOutput> {
    // Step 1: Validate the order
    let validated_order = ctx.run("validate_order", validate_order(input)).await?;

    // Step 2: Calculate total
    let order_total = ctx
        .run("calculate_total", calculate_total(validated_order))
        .await?;

    // Step 3: Process payment
    let (order_with_payment, _payment_id) = ctx
        .run("process_payment", process_payment(order_total))
        .await?;

    // Step 4: Finalize order
    let result = ctx
        .run(
            "finalize_order",
            finalize_order((order_with_payment, _payment_id)),
        )
        .await?;

    Ok(result)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    println!("🚀 Kagzi Macros Example");
    println!("========================");
    println!();
    println!("This example demonstrates the #[kagzi_step] attribute macro");
    println!("which automatically adds tracing, logging, and error context.");
    println!();

    // Create test order
    let test_order = ProcessOrderInput {
        order_id: "ORD-12345".to_string(),
        customer_id: "CUST-67890".to_string(),
        items: vec![
            OrderItem {
                product_id: "PROD-001".to_string(),
                quantity: 2,
                price: 29.99,
            },
            OrderItem {
                product_id: "PROD-002".to_string(),
                quantity: 1,
                price: 49.99,
            },
        ],
    };

    println!("Test order: {}", test_order.order_id);
    println!("Items: {}", test_order.items.len());
    println!(
        "Total expected: ${:.2}",
        test_order
            .items
            .iter()
            .map(|i| i.price * i.quantity as f64)
            .sum::<f64>()
    );
    println!();

    println!("Step functions defined with #[kagzi_step]:");
    println!("  - validate_order");
    println!("  - calculate_total");
    println!("  - process_payment");
    println!("  - finalize_order");
    println!();

    println!("Each macro adds:");
    println!("  ✓ Automatic tracing spans");
    println!("  ✓ Input/output logging at debug level");
    println!("  ✓ Error context enrichment");
    println!();

    println!("To run with actual workflow execution:");
    println!("1. Start the Kagzi server");
    println!("2. Register the process_order_workflow with a Worker");
    println!("3. Trigger workflows via the Client");
    println!();

    // Note: In a real application, you would:
    // 1. Create a Worker and register the workflow
    // 2. Create a Client to trigger the workflow
    //
    // For this demo, we're just showing that the macros compile correctly
    println!("✅ Macros compile successfully!");
    println!("✅ Step functions are ready to use with Kagzi workflows!");

    Ok(())
}
