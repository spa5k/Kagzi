//! Example demonstrating the `kagzi_workflow!` procedural macro.
//!
//! This example shows how the workflow macro provides a clean syntax
//! for defining workflows with a built-in `run!` helper macro.

use std::time::Duration;

use kagzi::WorkflowContext;
use kagzi_macros::{kagzi_step, kagzi_workflow};
use serde::{Deserialize, Serialize};

// Define input and output types
#[derive(Debug, Serialize, Deserialize, Clone)]
struct OrderRequest {
    order_id: String,
    customer_id: String,
    items: Vec<OrderItem>,
    shipping_address: Address,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct OrderItem {
    product_id: String,
    quantity: u32,
    price: f64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct Address {
    street: String,
    city: String,
    country: String,
    postal_code: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct OrderConfirmation {
    order_id: String,
    status: String,
    total_amount: f64,
    tax_amount: f64,
    shipping_cost: f64,
    estimated_delivery: chrono::DateTime<chrono::Utc>,
}

// Step functions enhanced with #[kagzi_step] attribute for automatic tracing and logging
#[kagzi_step]
async fn calculate_total(order: OrderRequest) -> anyhow::Result<(f64, f64)> {
    let subtotal: f64 = order
        .items
        .iter()
        .map(|item| item.price * item.quantity as f64)
        .sum();

    let tax = subtotal * 0.1; // 10% tax

    Ok((subtotal, tax))
}

#[kagzi_step]
async fn calculate_shipping(order: OrderRequest) -> anyhow::Result<f64> {
    // Simple shipping calculation based on country
    let cost = match order.shipping_address.country.as_str() {
        "US" => 10.0,
        "CA" => 15.0,
        _ => 25.0,
    };

    Ok(cost)
}

#[kagzi_step]
async fn reserve_inventory(order: OrderRequest) -> anyhow::Result<String> {
    // Simulate inventory reservation
    tokio::time::sleep(Duration::from_millis(300)).await;

    let reservation_id = format!(
        "inv-{}-{}",
        order.order_id,
        uuid::Uuid::new_v4()
            .to_string()
            .chars()
            .take(8)
            .collect::<String>()
    );

    tracing::info!(
        order_id = %order.order_id,
        reservation_id = %reservation_id,
        "Inventory reserved"
    );

    Ok(reservation_id)
}

#[kagzi_step]
async fn process_payment((order, total): (OrderRequest, f64)) -> anyhow::Result<String> {
    // Simulate payment processing
    tokio::time::sleep(Duration::from_millis(500)).await;

    let payment_id = format!(
        "pay-{}-{}",
        order.order_id,
        uuid::Uuid::new_v4()
            .to_string()
            .chars()
            .take(8)
            .collect::<String>()
    );

    tracing::info!(
        order_id = %order.order_id,
        payment_id = %payment_id,
        amount = %total,
        "Payment processed"
    );

    Ok(payment_id)
}

#[kagzi_step]
async fn schedule_shipment(
    (order, _shipping_cost): (OrderRequest, f64),
) -> anyhow::Result<chrono::DateTime<chrono::Utc>> {
    // Simulate shipment scheduling
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Estimate delivery (2-5 business days from now)
    let delivery_date = chrono::Utc::now() + chrono::Duration::days(5);

    tracing::info!(
        order_id = %order.order_id,
        delivery_date = %delivery_date,
        "Shipment scheduled"
    );

    Ok(delivery_date)
}

#[kagzi_step]
async fn send_confirmation_email(
    (order, _confirmation): (OrderRequest, OrderConfirmation),
) -> anyhow::Result<String> {
    // Simulate sending email
    tokio::time::sleep(Duration::from_millis(100)).await;

    let email_id = format!(
        "email-{}-{}",
        order.order_id,
        uuid::Uuid::new_v4()
            .to_string()
            .chars()
            .take(8)
            .collect::<String>()
    );

    tracing::info!(
        order_id = %order.order_id,
        email_id = %email_id,
        customer_id = %order.customer_id,
        "Confirmation email sent"
    );

    Ok(email_id)
}

// Workflow using kagzi_workflow! macro for cleaner syntax
kagzi_workflow! {
    /// Process an e-commerce order with inventory reservation and payment
    pub async fn process_order_with_macro(
        mut ctx: WorkflowContext,
        input: OrderRequest,
    ) -> anyhow::Result<OrderConfirmation> {
        // The run! macro is automatically defined by kagzi_workflow!
        // It provides a much cleaner syntax for step execution

        // Calculate order totals
        let (subtotal, tax) = run!("calculate_total", calculate_total(input.clone()));
        let shipping_cost = run!("calculate_shipping", calculate_shipping(input.clone()));

        // Reserve inventory
        let _inventory_id = run!("reserve_inventory", reserve_inventory(input.clone()));

        // Process payment
        let total = subtotal + tax + shipping_cost;
        let _payment_id = run!("process_payment", process_payment((input.clone(), total)));

        // Schedule shipment
        let delivery_date = run!("schedule_shipment", schedule_shipment((input.clone(), shipping_cost)));

        // Create confirmation
        let confirmation = OrderConfirmation {
            order_id: input.order_id.clone(),
            status: "confirmed".to_string(),
            total_amount: total,
            tax_amount: tax,
            shipping_cost,
            estimated_delivery: delivery_date,
        };

        // Send confirmation email
        let _email_id = run!("send_email", send_confirmation_email((input, confirmation.clone())));

        // Return final result
        Ok(confirmation)
    }
}

// Another example showing different return types
kagzi_workflow! {
    /// Process a refund workflow
    pub async fn process_refund(
        mut ctx: WorkflowContext,
        _order_id: String,
    ) -> anyhow::Result<String> {
        // Validate refund eligibility
        let eligible = run!("check_eligibility", async {
            tokio::time::sleep(Duration::from_millis(100)).await;
            Ok(true)
        });

        if !eligible {
            anyhow::bail!("Order not eligible for refund");
        }

        // Process refund
        let refund_id = run!("process_refund", async {
            tokio::time::sleep(Duration::from_millis(300)).await;
            Ok(format!("refund-{}", uuid::Uuid::new_v4()))
        });

        // Send notification
        run!("send_notification", async {
            tokio::time::sleep(Duration::from_millis(50)).await;
            Ok(())
        });

        Ok(refund_id)
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    println!("🚀 Kagzi Workflow Macro Example");
    println!("===============================");
    println!();
    println!("This example demonstrates the kagzi_workflow! macro");
    println!("which provides a cleaner syntax with built-in run! helper.");
    println!();

    // Create test order
    let order = OrderRequest {
        order_id: "ORD-2024-001".to_string(),
        customer_id: "CUST-123".to_string(),
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
        shipping_address: Address {
            street: "123 Main St".to_string(),
            city: "New York".to_string(),
            country: "US".to_string(),
            postal_code: "10001".to_string(),
        },
    };

    println!("Processing order: {}", order.order_id);
    println!("Items: {}", order.items.len());
    println!(
        "Shipping to: {}, {}",
        order.shipping_address.city, order.shipping_address.country
    );
    println!();

    println!("kagzi_workflow! macro syntax (clean):");
    println!("```rust");
    println!("let result = run!(\"step_name\", async_function(input));");
    println!("```");
    println!();

    println!("Benefits of kagzi_workflow!:");
    println!("  ✓ Cleaner, more readable syntax");
    println!("  ✓ Automatic workflow-level tracing");
    println!("  ✓ Built-in run! helper for step execution");
    println!("  ✓ Less repetitive code");
    println!("  ✓ Automatic error propagation with ? operator");
    println!();

    // Demo a simple async block to show the macro compilation
    println!("The macros compile and work correctly!");
    println!("See the code in examples/13_kagzi_workflow_macro.rs");
    println!();

    println!("To test with actual workflow execution:");
    println!("1. Start the Kagzi server");
    println!("2. Register these workflow functions with a Worker");
    println!("3. Trigger workflows via the Client");
    println!();

    Ok(())
}
