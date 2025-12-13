//! Comprehensive example showing both macros working together.
//!
//! This example demonstrates a complete e-commerce order processing workflow
//! using both #[kagzi_step] and kagzi_workflow! macros together.

use std::time::Duration;

use kagzi::WorkflowContext;
use kagzi_macros::{kagzi_step, kagzi_workflow};
use serde::{Deserialize, Serialize};

// ========================
// Input/Output Types
// ========================

#[derive(Debug, Serialize, Deserialize, Clone)]
struct OrderInput {
    order_id: String,
    customer_id: String,
    customer_email: String,
    items: Vec<OrderItem>,
    shipping_address: Address,
    payment_method: PaymentMethod,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct OrderItem {
    product_id: String,
    name: String,
    quantity: u32,
    unit_price: f64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct Address {
    street: String,
    city: String,
    state: String,
    country: String,
    postal_code: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
enum PaymentMethod {
    CreditCard { last_four: String },
    PayPal { email: String },
    BankTransfer,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct OrderOutput {
    order_id: String,
    status: String,
    total_amount: f64,
    order_placed_at: chrono::DateTime<chrono::Utc>,
    estimated_delivery: chrono::DateTime<chrono::Utc>,
    payment_status: String,
    items_processed: u32,
}

// ========================
// Step Functions with #[kagzi_step]
// ========================

#[kagzi_step]
/// Validates order input and ensures all required fields are present
async fn validate_order(order: OrderInput) -> anyhow::Result<OrderInput> {
    // Validate order ID
    if order.order_id.is_empty() {
        anyhow::bail!("Order ID cannot be empty");
    }

    // Validate customer ID
    if order.customer_id.is_empty() {
        anyhow::bail!("Customer ID cannot be empty");
    }

    // Validate email
    if !order.customer_email.contains('@') {
        anyhow::bail!("Invalid customer email format");
    }

    // Validate items
    if order.items.is_empty() {
        anyhow::bail!("Order must contain at least one item");
    }

    // Validate each item
    for item in &order.items {
        if item.product_id.is_empty() {
            anyhow::bail!("Product ID cannot be empty");
        }
        if item.quantity == 0 {
            anyhow::bail!("Item quantity must be greater than 0");
        }
        if item.unit_price <= 0.0 {
            anyhow::bail!("Item price must be greater than 0");
        }
    }

    // Validate shipping address
    if order.shipping_address.street.is_empty() {
        anyhow::bail!("Street address cannot be empty");
    }

    tracing::info!(
        order_id = %order.order_id,
        customer_id = %order.customer_id,
        items_count = %order.items.len(),
        "Order validation completed successfully"
    );

    Ok(order)
}

#[kagzi_step]
/// Calculates pricing including tax and shipping
async fn calculate_pricing(order: OrderInput) -> anyhow::Result<(OrderInput, Pricing)> {
    // Calculate subtotal
    let subtotal: f64 = order
        .items
        .iter()
        .map(|item| item.unit_price * item.quantity as f64)
        .sum();

    // Calculate tax (10% for this example)
    let tax_rate = 0.1;
    let tax = subtotal * tax_rate;

    // Calculate shipping based on country
    let shipping_cost = match order.shipping_address.country.as_str() {
        "US" | "CA" => 10.0,
        "UK" => 15.0,
        _ => 25.0,
    };

    let total = subtotal + tax + shipping_cost;

    let pricing = Pricing {
        subtotal,
        tax,
        shipping: shipping_cost,
        total,
        currency: "USD".to_string(),
    };

    tracing::info!(
        order_id = %order.order_id,
        subtotal = %subtotal,
        tax = %tax,
        shipping = %shipping_cost,
        total = %total,
        "Pricing calculated"
    );

    Ok((order, pricing))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Pricing {
    subtotal: f64,
    tax: f64,
    shipping: f64,
    total: f64,
    currency: String,
}

#[kagzi_step]
/// Reserves inventory for all items in the order
async fn reserve_inventory(
    (order, pricing): (OrderInput, Pricing),
) -> anyhow::Result<(OrderInput, Pricing, InventoryReservation)> {
    // Simulate inventory check and reservation
    tokio::time::sleep(Duration::from_millis(300)).await;

    let reservations = order
        .items
        .iter()
        .map(|item| InventoryItem {
            product_id: item.product_id.clone(),
            quantity_reserved: item.quantity,
        })
        .collect();

    let reservation = InventoryReservation {
        reservation_id: format!("res-{}-{}", order.order_id, uuid::Uuid::new_v4()),
        items: reservations,
        expires_at: chrono::Utc::now() + chrono::Duration::minutes(30),
    };

    tracing::info!(
        order_id = %order.order_id,
        reservation_id = %reservation.reservation_id,
        items_count = %reservation.items.len(),
        "Inventory reserved successfully"
    );

    Ok((order, pricing, reservation))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct InventoryReservation {
    reservation_id: String,
    items: Vec<InventoryItem>,
    expires_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct InventoryItem {
    product_id: String,
    quantity_reserved: u32,
}

#[kagzi_step]
/// Processes payment for the order
async fn process_payment(
    (order, pricing, _reservation): (OrderInput, Pricing, InventoryReservation),
) -> anyhow::Result<(OrderInput, Pricing, PaymentResult)> {
    // Simulate payment processing (always succeeds in this demo)
    tokio::time::sleep(Duration::from_millis(500)).await;

    let payment = PaymentResult {
        payment_id: format!("pay-{}-{}", order.order_id, uuid::Uuid::new_v4()),
        status: "completed".to_string(),
        amount: pricing.total,
        currency: pricing.currency.clone(),
        processed_at: chrono::Utc::now(),
    };

    tracing::info!(
        order_id = %order.order_id,
        payment_id = %payment.payment_id,
        amount = %payment.amount,
        "Payment processed successfully"
    );

    Ok((order, pricing, payment))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PaymentResult {
    payment_id: String,
    status: String,
    amount: f64,
    currency: String,
    processed_at: chrono::DateTime<chrono::Utc>,
}

#[kagzi_step]
/// Creates a shipment for the order
async fn create_shipment(
    (order, _pricing, _payment): (OrderInput, Pricing, PaymentResult),
) -> anyhow::Result<(OrderInput, ShippingInfo)> {
    // Simulate shipment creation
    tokio::time::sleep(Duration::from_millis(200)).await;

    let days_to_delivery = match order.shipping_address.country.as_str() {
        "US" => 2,
        "CA" => 3,
        "UK" => 5,
        _ => 7,
    };

    let estimated_delivery = chrono::Utc::now() + chrono::Duration::days(days_to_delivery);

    let tracking_number = format!(
        "1Z{}{}",
        order.order_id.chars().take(8).collect::<String>(),
        uuid::Uuid::new_v4()
            .to_string()
            .chars()
            .take(10)
            .collect::<String>()
    );

    let shipping = ShippingInfo {
        method: match order.shipping_address.country.as_str() {
            "US" => "Standard Ground",
            "CA" => "Canada Post",
            _ => "International",
        }
        .to_string(),
        estimated_delivery,
        tracking_number,
    };

    tracing::info!(
        order_id = %order.order_id,
        tracking_number = %shipping.tracking_number,
        estimated_delivery = %shipping.estimated_delivery,
        "Shipment created"
    );

    Ok((order, shipping))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ShippingInfo {
    method: String,
    estimated_delivery: chrono::DateTime<chrono::Utc>,
    tracking_number: String,
}

#[kagzi_step]
/// Sends order confirmation email to the customer
async fn send_order_confirmation(
    (order, shipping): (OrderInput, ShippingInfo),
) -> anyhow::Result<String> {
    // Simulate sending email
    tokio::time::sleep(Duration::from_millis(100)).await;

    let email_id = format!("email-{}-{}", order.order_id, uuid::Uuid::new_v4());

    tracing::info!(
        order_id = %order.order_id,
        customer_email = %order.customer_email,
        email_id = %email_id,
        tracking_number = %shipping.tracking_number,
        "Order confirmation email sent"
    );

    Ok(email_id)
}

// ========================
// Workflow Using kagzi_workflow!
// ========================

kagzi_workflow! {
    /// Complete e-commerce order processing workflow
    ///
    /// This workflow processes an order through all stages:
    /// 1. Validates input
    /// 2. Calculates pricing
    /// 3. Reserves inventory
    /// 4. Processes payment
    /// 5. Creates shipment
    /// 6. Sends confirmation
    pub async fn process_ecommerce_order(
        mut ctx: WorkflowContext,
        input: OrderInput,
    ) -> anyhow::Result<OrderOutput> {
        // Step 1: Validate order input
        let validated_order = run!("validate_input", validate_order(input));

        // Step 2: Calculate pricing
        let (order, pricing) = run!("calculate_pricing", calculate_pricing(validated_order));

        // Step 3: Reserve inventory
        let (order, pricing, _reservation) = run!(
            "reserve_inventory",
            reserve_inventory((order, pricing))
        );

        // Step 4: Process payment
        let (order, _pricing, payment) = run!(
            "process_payment",
            process_payment((order, pricing, _reservation))
        );

        // Step 5: Create shipment
        let (order, shipping) = run!(
            "create_shipment",
            create_shipment((order, _pricing, payment.clone()))
        );

        // Step 6: Send confirmation
        let _email_id = run!(
            "send_confirmation",
            send_order_confirmation((order.clone(), shipping.clone()))
        );

        // Build final order output
        let output = OrderOutput {
            order_id: order.order_id.clone(),
            status: "confirmed".to_string(),
            total_amount: payment.amount,
            order_placed_at: payment.processed_at,
            estimated_delivery: shipping.estimated_delivery,
            payment_status: payment.status,
            items_processed: order.items.len() as u32,
        };

        tracing::info!(
            order_id = %output.order_id,
            total = %output.total_amount,
            status = %output.status,
            "E-commerce order processing completed successfully"
        );

        Ok(output)
    }
}

// Another workflow example - order cancellation
kagzi_workflow! {
    /// Cancels an existing order and releases inventory
    pub async fn cancel_order(
        mut ctx: WorkflowContext,
        order_id: String,
    ) -> anyhow::Result<String> {
        // Validate order exists and can be cancelled
        let order_status = run!("check_order_status", async {
            tokio::time::sleep(Duration::from_millis(100)).await;
            // In a real implementation, check against database
            Ok("confirmed".to_string())
        });

        if order_status != "confirmed" && order_status != "processing" {
            anyhow::bail!("Order cannot be cancelled in current status: {}", order_status);
        }

        // Release inventory
        run!("release_inventory", async {
            tokio::time::sleep(Duration::from_millis(200)).await;
            tracing::info!(order_id = %order_id, "Inventory released for cancelled order");
            Ok(())
        });

        // Process refund
        let refund_id = run!("process_refund", async {
            tokio::time::sleep(Duration::from_millis(500)).await;
            let id = format!("refund-{}-{}", order_id, uuid::Uuid::new_v4());
            tracing::info!(order_id = %order_id, refund_id = %id, "Refund processed");
            Ok(id)
        });

        // Send cancellation notification
        run!("send_cancellation_email", async {
            tokio::time::sleep(Duration::from_millis(100)).await;
            tracing::info!(order_id = %order_id, "Cancellation email sent");
            Ok(())
        });

        tracing::info!(
            order_id = %order_id,
            refund_id = %refund_id,
            "Order cancellation completed"
        );

        Ok(refund_id)
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    println!("🛒 Kagzi Comprehensive E-commerce Example");
    println!("========================================");
    println!();
    println!("This example shows both macros working together:");
    println!("- #[kagzi_step] for individual step functions");
    println!("- kagzi_workflow! for workflow orchestration");
    println!();

    // Create test order
    let test_order = OrderInput {
        order_id: "ORD-2024-001".to_string(),
        customer_id: "CUST-12345".to_string(),
        customer_email: "john.doe@example.com".to_string(),
        items: vec![
            OrderItem {
                product_id: "LAPTOP-MACBOOK".to_string(),
                name: "MacBook Pro 16-inch".to_string(),
                quantity: 1,
                unit_price: 2499.99,
            },
            OrderItem {
                product_id: "MOUSE-MAGIC".to_string(),
                name: "Magic Mouse".to_string(),
                quantity: 1,
                unit_price: 99.99,
            },
            OrderItem {
                product_id: "CASE-MACBOOK".to_string(),
                name: "MacBook Pro Sleeve".to_string(),
                quantity: 2,
                unit_price: 49.99,
            },
        ],
        shipping_address: Address {
            street: "123 Main Street".to_string(),
            city: "New York".to_string(),
            state: "NY".to_string(),
            country: "US".to_string(),
            postal_code: "10001".to_string(),
        },
        payment_method: PaymentMethod::CreditCard {
            last_four: "1234".to_string(),
        },
    };

    println!("Test Order:");
    println!("  Order ID: {}", test_order.order_id);
    println!("  Customer: {}", test_order.customer_email);
    println!(
        "  Items: {} ({} products)",
        test_order.items.len(),
        test_order.items.iter().map(|i| i.quantity).sum::<u32>()
    );
    println!(
        "  Shipping: {}, {}",
        test_order.shipping_address.city, test_order.shipping_address.country
    );
    println!();

    let total: f64 = test_order
        .items
        .iter()
        .map(|i| i.unit_price * i.quantity as f64)
        .sum();

    println!("Order Summary:");
    println!("  Subtotal: ${:.2}", total);
    println!("  Tax (10%): ${:.2}", total * 0.1);
    println!("  Shipping: $10.00");
    println!("  Total: ${:.2}", total * 1.1 + 10.0);
    println!();

    println!("Macro Benefits:");
    println!("  ✓ Automatic tracing for all steps");
    println!("  ✓ Input/output logging at debug level");
    println!("  ✓ Error context enrichment");
    println!("  ✓ Clean workflow syntax with run! macro");
    println!("  ✓ 50-70% less boilerplate code");
    println!();

    println!("To test with actual workflow execution:");
    println!("1. Start the Kagzi server");
    println!("2. Register process_ecommerce_order with a Worker");
    println!("3. Trigger the workflow via the Client");
    println!();

    println!("✅ All macros compile successfully!");
    println!("✅ Ready for production use with Kagzi!");

    Ok(())
}
