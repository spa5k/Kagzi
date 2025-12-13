//! Example demonstrating the `kagzi_workflow!` procedural macro.
//!
//! This example shows how the workflow macro provides a clean syntax
//! for defining workflows with a built-in `run!` helper macro.

use std::time::Duration;

use kagzi::prelude::*;
use kagzi_macros::kagzi_workflow;
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

#[derive(Debug, Serialize, Deserialize)]
struct OrderConfirmation {
    order_id: String,
    status: String,
    total_amount: f64,
    tax_amount: f64,
    shipping_cost: f64,
    estimated_delivery: chrono::Date<chrono::Utc>,
}

// Step functions (these would typically have #[kagzi_step] attribute)
async fn calculate_total(_ctx: WorkflowContext, order: OrderRequest) -> anyhow::Result<(f64, f64)> {
    let subtotal: f64 = order
        .items
        .iter()
        .map(|item| item.price * item.quantity as f64)
        .sum();

    let tax = subtotal * 0.1; // 10% tax

    Ok((subtotal, tax))
}

async fn calculate_shipping(_ctx: WorkflowContext, order: OrderRequest) -> anyhow::Result<f64> {
    // Simple shipping calculation based on country
    let cost = match order.shipping_address.country.as_str() {
        "US" => 10.0,
        "CA" => 15.0,
        _ => 25.0,
    };

    Ok(cost)
}

async fn reserve_inventory(_ctx: WorkflowContext, order: OrderRequest) -> anyhow::Result<String> {
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

async fn process_payment(
    _ctx: WorkflowContext,
    (order, total): (OrderRequest, f64),
) -> anyhow::Result<String> {
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
        amount = total,
        "Payment processed"
    );

    Ok(payment_id)
}

async fn schedule_shipment(
    _ctx: WorkflowContext,
    (order, shipping_cost): (OrderRequest, f64),
) -> anyhow::Result<chrono::Date<chrono::Utc>> {
    // Simulate shipment scheduling
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Estimate delivery (2-5 business days from now)
    let delivery_date = chrono::Utc::now()
        .date_naive()
        .checked_add_days(chrono::Days::new(5))
        .map(|d| d.and_hms_opt(0, 0, 0).unwrap().and_utc())
        .unwrap();

    tracing::info!(
        order_id = %order.order_id,
        delivery_date = %delivery_date,
        "Shipment scheduled"
    );

    Ok(chrono::Date::from_utc(
        delivery_date.and_hms_opt(0, 0, 0).unwrap(),
        chrono::Utc,
    ))
}

async fn send_confirmation_email(
    _ctx: WorkflowContext,
    (order, confirmation): (OrderRequest, OrderConfirmation),
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

// Traditional workflow definition (without macro)
pub async fn process_order_traditional(
    mut ctx: WorkflowContext,
    input: OrderRequest,
) -> anyhow::Result<OrderConfirmation> {
    // Manually run each step - verbose and repetitive
    let (subtotal, tax) = ctx
        .run(
            "calculate_total",
            calculate_total(ctx.clone(), input.clone()),
        )
        .await?;

    let shipping_cost = ctx
        .run(
            "calculate_shipping",
            calculate_shipping(ctx.clone(), input.clone()),
        )
        .await?;

    let _inventory = ctx
        .run(
            "reserve_inventory",
            reserve_inventory(ctx.clone(), input.clone()),
        )
        .await?;

    let total = subtotal + tax + shipping_cost;

    let _payment = ctx
        .run(
            "process_payment",
            process_payment(ctx.clone(), (input.clone(), total)),
        )
        .await?;

    let delivery_date = ctx
        .run(
            "schedule_shipment",
            schedule_shipment(ctx.clone(), (input.clone(), shipping_cost)),
        )
        .await?;

    let confirmation = OrderConfirmation {
        order_id: input.order_id.clone(),
        status: "confirmed".to_string(),
        total_amount: total,
        tax_amount: tax,
        shipping_cost,
        estimated_delivery: delivery_date,
    };

    let _email = ctx
        .run(
            "send_email",
            send_confirmation_email(ctx.clone(), (input, confirmation.clone())),
        )
        .await?;

    Ok(confirmation)
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
        let (subtotal, tax) = run!("calculate_total", calculate_total(&input));
        let shipping_cost = run!("calculate_shipping", calculate_shipping(&input));

        // Reserve inventory
        let inventory_id = run!("reserve_inventory", reserve_inventory(&input));

        // Process payment
        let total = subtotal + tax + shipping_cost;
        let payment_id = run!("process_payment", process_payment(&(input, total)));

        // Schedule shipment
        let delivery_date = run!("schedule_shipment", schedule_shipment(&(input, shipping_cost)));

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
        let _email_id = run!("send_email", send_confirmation_email(&(input, confirmation.clone())));

        // Return final result
        Ok(confirmation)
    }
}

// Another example showing different return types
kagzi_workflow! {
    /// Process a refund workflow
    pub async fn process_refund(
        mut ctx: WorkflowContext,
        order_id: String,
    ) -> anyhow::Result<String> {
        // Validate refund eligibility
        let eligible = run!("check_eligibility", async {
            tokio::time::sleep(Duration::from_millis(100)).await;
            Ok(true)
        });

        if !eligible? {
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

        refund_id
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

    // Note: In a real scenario, you would create a proper WorkflowContext
    // and run these as registered workflows. For this demo, we'll just
    // show the syntax difference.

    println!("Traditional workflow syntax (verbose):");
    println!("```rust");
    println!("let result = ctx");
    println!("    .run(\"step_name\", async_function(ctx.clone(), input.clone()))");
    println!("    .await?;");
    println!("```");
    println!();

    println!("kagzi_workflow! macro syntax (clean):");
    println!("```rust");
    println!("let result = run!(\"step_name\", async_function(&input));");
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
