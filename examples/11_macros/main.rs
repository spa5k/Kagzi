//! Example demonstrating the new Kagzi macros.
//!
//! This example shows how the simplified macros make workflow definitions
//! more readable and observable.

use std::time::Duration;

use kagzi::prelude::*;
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

#[derive(Debug, Serialize, Deserialize)]
struct ProcessOrderOutput {
    order_id: String,
    status: String,
    total_amount: f64,
    processed_at: chrono::DateTime<chrono::Utc>,
}

// Step functions with the #[kagzi_step] macro for observability
#[kagzi_step]
/// Validates an order and calculates the total amount
async fn validate_order(
    _ctx: WorkflowContext,
    input: ProcessOrderInput,
) -> anyhow::Result<ProcessOrderInput> {
    let validated = input.clone();

    if validated.items.is_empty() {
        anyhow::bail!("Order cannot be empty");
    }

    if validated.customer_id.is_empty() {
        anyhow::bail!("Customer ID is required");
    }

    // Calculate total amount
    let total: f64 = validated
        .items
        .iter()
        .map(|item| item.price * item.quantity as f64)
        .sum();

    if total <= 0.0 {
        anyhow::bail!("Order amount must be positive");
    }

    tracing::info!(order_id = %validated.order_id, total = total, "Order validated");

    Ok(validated)
}

#[kagzi_step]
/// Simulates inventory reservation
async fn reserve_inventory(
    _ctx: WorkflowContext,
    input: ProcessOrderInput,
) -> anyhow::Result<String> {
    // Simulate processing time
    tokio::time::sleep(Duration::from_millis(300)).await;

    let reservation_id = format!("inv-{}", uuid::Uuid::new_v4());
    tracing::info!(
        order_id = %input.order_id,
        reservation_id = %reservation_id,
        "Inventory reserved"
    );

    Ok::<String, anyhow::Error>(reservation_id)
}

#[kagzi_step]
/// Simulates payment processing
async fn process_payment(
    _ctx: WorkflowContext,
    input: ProcessOrderInput,
) -> anyhow::Result<String> {
    // Simulate processing time
    tokio::time::sleep(Duration::from_millis(500)).await;

    let payment_id = format!("pay-{}", uuid::Uuid::new_v4());
    let total: f64 = input
        .items
        .iter()
        .map(|item| item.price * item.quantity as f64)
        .sum();

    tracing::info!(
        order_id = %input.order_id,
        payment_id = %payment_id,
        amount = total,
        "Payment processed"
    );

    Ok::<String, anyhow::Error>(payment_id)
}

// Test function to demonstrate the step macro
#[tokio::main]
async fn test_step_macro() -> anyhow::Result<()> {
    // This demonstrates that the step functions are just enhanced async functions
    // They can be called directly (assuming a WorkflowContext is provided)

    println!("✅ Step macro demonstration:");
    println!("  The #[kagzi_step] attribute adds:");
    println!("  - Automatic tracing spans");
    println!("  - Input/output logging at debug level");
    println!("  - Error context enrichment");
    println!();

    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize logging with debug level to see macro-generated logs
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    println!("🚀 Kagzi Macros Example");
    println!("========================");
    println!("Demonstrating the new kagzi_step macro");
    println!();

    println!("✅ Step functions defined with #[kagzi_step]:");
    println!("   - validate_order");
    println!("   - reserve_inventory");
    println!("   - process_payment");
    println!();
    println!("Key features demonstrated:");
    println!("  1. #[kagzi_step] attribute adds automatic tracing and error context");
    println!("  2. Structured logging with input/output at debug level");
    println!("  3. Error context enrichment for better debugging");
    println!();
    println!("The kagzi_step macro:");
    println!("  ✓ Adds tracing spans with step name");
    println!("  ✓ Logs inputs/outputs at debug level");
    println!("  ✓ Enriches errors with step context");
    println!("  ✓ Preserves function behavior completely");
    println!();
    println!("To see the debug logging, set RUST_LOG=debug:");
    println!("  RUST_LOG=debug cargo run --example 11_macros");

    Ok(())
}
