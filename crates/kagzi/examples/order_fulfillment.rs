//! Example: Order Fulfillment Workflow
//!
//! This example demonstrates:
//! - Multi-step e-commerce order processing
//! - Error handling with step memoization
//! - Durable sleep for waiting periods
//! - Complex business logic with multiple stages
//!
//! Run with: cargo run --example order_fulfillment

use kagzi::{Kagzi, WorkflowContext};
use serde::{Deserialize, Serialize};
use std::env;
use std::time::Duration;

#[derive(Debug, Serialize, Deserialize, Clone)]
struct OrderInput {
    order_id: String,
    customer_id: String,
    items: Vec<OrderItem>,
    total_amount: f64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct OrderItem {
    product_id: String,
    quantity: u32,
    price: f64,
}

#[derive(Debug, Serialize, Deserialize)]
struct PaymentResult {
    transaction_id: String,
    status: String,
    charged_amount: f64,
}

#[derive(Debug, Serialize, Deserialize)]
struct InventoryReservation {
    reservation_id: String,
    items: Vec<ReservedItem>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ReservedItem {
    product_id: String,
    quantity: u32,
    warehouse: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct ShipmentInfo {
    tracking_number: String,
    carrier: String,
    estimated_delivery: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct OrderFulfillmentResult {
    order_id: String,
    status: String,
    payment_transaction_id: String,
    tracking_number: String,
    total_processed: f64,
}

/// Order fulfillment workflow
async fn order_fulfillment_workflow(
    ctx: WorkflowContext,
    input: OrderInput,
) -> anyhow::Result<OrderFulfillmentResult> {
    println!("🛒 Processing order: {}", input.order_id);
    println!("   Customer: {}", input.customer_id);
    println!("   Items: {}", input.items.len());
    println!("   Total: ${:.2}", input.total_amount);

    // Step 1: Validate order
    ctx.step("validate-order", async {
        println!("  ✓ Validating order...");

        // Simulate validation
        if input.items.is_empty() {
            anyhow::bail!("Order has no items");
        }

        if input.total_amount <= 0.0 {
            anyhow::bail!("Invalid order amount");
        }

        println!("     ✓ Order validation passed");
        Ok::<_, anyhow::Error>(())
    })
    .await?;

    // Step 2: Process payment
    let payment: PaymentResult = ctx
        .step("process-payment", async {
            println!("  💳 Processing payment of ${:.2}...", input.total_amount);

            // Simulate payment processing
            tokio::time::sleep(Duration::from_secs(2)).await;

            let payment = PaymentResult {
                transaction_id: format!("txn_{}", uuid::Uuid::new_v4()),
                status: "APPROVED".to_string(),
                charged_amount: input.total_amount,
            };

            println!("     ✓ Payment approved: {}", payment.transaction_id);

            Ok::<_, anyhow::Error>(payment)
        })
        .await?;

    // Step 3: Reserve inventory
    let _reservation: InventoryReservation = ctx
        .step("reserve-inventory", async {
            println!(
                "  📦 Reserving inventory for {} items...",
                input.items.len()
            );

            // Simulate inventory reservation
            tokio::time::sleep(Duration::from_secs(1)).await;

            let reserved_items: Vec<ReservedItem> = input
                .items
                .iter()
                .map(|item| ReservedItem {
                    product_id: item.product_id.clone(),
                    quantity: item.quantity,
                    warehouse: "WAREHOUSE_A".to_string(),
                })
                .collect();

            let reservation = InventoryReservation {
                reservation_id: format!("rsv_{}", uuid::Uuid::new_v4()),
                items: reserved_items,
            };

            println!("     ✓ Inventory reserved: {}", reservation.reservation_id);

            Ok::<_, anyhow::Error>(reservation)
        })
        .await?;

    // Step 4: Wait for warehouse processing (durable sleep)
    println!("  ⏰ Waiting for warehouse to prepare shipment (3 seconds)...");
    ctx.sleep("warehouse-preparation", Duration::from_secs(3))
        .await?;
    println!("     ✓ Warehouse preparation complete");

    // Step 5: Create shipment
    let shipment: ShipmentInfo = ctx
        .step("create-shipment", async {
            println!("  🚚 Creating shipment...");

            // Simulate shipment creation
            tokio::time::sleep(Duration::from_secs(1)).await;

            let shipment = ShipmentInfo {
                tracking_number: format!(
                    "TRK{}",
                    &uuid::Uuid::new_v4()
                        .to_string()
                        .replace("-", "")
                        .to_uppercase()[..12]
                ),
                carrier: "FastShip Express".to_string(),
                estimated_delivery: chrono::Utc::now()
                    .checked_add_signed(chrono::Duration::days(3))
                    .unwrap()
                    .format("%Y-%m-%d")
                    .to_string(),
            };

            println!("     ✓ Shipment created: {}", shipment.tracking_number);
            println!("     ✓ Carrier: {}", shipment.carrier);
            println!("     ✓ Estimated delivery: {}", shipment.estimated_delivery);

            Ok::<_, anyhow::Error>(shipment)
        })
        .await?;

    // Step 6: Send confirmation email
    ctx.step("send-confirmation", async {
        println!("  📧 Sending order confirmation email...");

        // Simulate email sending
        tokio::time::sleep(Duration::from_millis(500)).await;

        println!("     ✓ Confirmation email sent to customer");

        Ok::<_, anyhow::Error>(())
    })
    .await?;

    // Step 7: Update order status
    ctx.step("update-order-status", async {
        println!("  📝 Updating order status to SHIPPED...");

        // Simulate database update
        tokio::time::sleep(Duration::from_millis(300)).await;

        println!("     ✓ Order status updated");

        Ok::<_, anyhow::Error>(())
    })
    .await?;

    let result = OrderFulfillmentResult {
        order_id: input.order_id.clone(),
        status: "SHIPPED".to_string(),
        payment_transaction_id: payment.transaction_id,
        tracking_number: shipment.tracking_number,
        total_processed: payment.charged_amount,
    };

    println!("✅ Order {} fulfilled successfully!", input.order_id);

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
        .register_workflow("order-fulfillment", order_fulfillment_workflow)
        .await;

    // Sample order
    let sample_order = OrderInput {
        order_id: "ORD-12345".to_string(),
        customer_id: "CUST-98765".to_string(),
        items: vec![
            OrderItem {
                product_id: "PROD-001".to_string(),
                quantity: 2,
                price: 29.99,
            },
            OrderItem {
                product_id: "PROD-042".to_string(),
                quantity: 1,
                price: 49.99,
            },
        ],
        total_amount: 109.97,
    };

    println!("Starting order fulfillment workflow...");
    let handle = kagzi
        .start_workflow("order-fulfillment", sample_order)
        .await?;

    println!("Workflow started: {}\n", handle.run_id());

    // Start worker in background
    let worker = kagzi.create_worker();
    tokio::spawn(async move {
        worker.start().await.expect("Worker failed");
    });

    // Wait for result
    println!("Processing order...\n");
    let result_value = handle.result().await?;
    let result: OrderFulfillmentResult = serde_json::from_value(result_value)?;
    println!("\n=== Order Fulfillment Complete ===");
    println!("{}", serde_json::to_string_pretty(&result)?);

    Ok(())
}
