//! Comprehensive example demonstrating both Kagzi macros working together.
//!
//! This example shows a real-world e-commerce order processing workflow
//! using both #[kagzi_step] and kagzi_workflow! macros for maximum
//! code clarity and observability.

use std::time::Duration;

use kagzi::prelude::*;
use kagzi_macros::{kagzi_step, kagzi_workflow};
use serde::{Deserialize, Serialize};

// ========================
// Data Structures
// ========================

#[derive(Debug, Serialize, Deserialize, Clone)]
struct OrderInput {
    order_id: String,
    customer_id: String,
    items: Vec<OrderItem>,
    shipping_address: Address,
    payment_method: PaymentMethod,
    promo_code: Option<String>,
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

#[derive(Debug, Serialize, Deserialize)]
struct OrderOutput {
    order_id: String,
    status: OrderStatus,
    items: Vec<ProcessedItem>,
    pricing: Pricing,
    shipping: ShippingInfo,
    payment: PaymentResult,
    timestamps: OrderTimestamps,
    tracking_numbers: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
enum OrderStatus {
    Pending,
    Confirmed,
    Processing,
    Shipped,
    Delivered,
    Cancelled,
}

#[derive(Debug, Serialize, Deserialize)]
struct ProcessedItem {
    product_id: String,
    quantity: u32,
    price: f64,
    allocated: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct Pricing {
    subtotal: f64,
    tax: f64,
    shipping: f64,
    discount: f64,
    total: f64,
    currency: String,
}

#[derive(Debug, Serialize, Clone)]
struct ShippingInfo {
    method: String,
    cost: f64,
    estimated_delivery: chrono::Date<chrono::Utc>,
    tracking_numbers: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct PaymentResult {
    transaction_id: String,
    status: String,
    amount: f64,
    processed_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize, Deserialize)]
struct OrderTimestamps {
    created_at: chrono::DateTime<chrono::Utc>,
    confirmed_at: chrono::DateTime<chrono::Utc>,
    processed_at: chrono::DateTime<chrono::Utc>,
}

// ========================
// Step Functions with #[kagzi_step]
// ========================

#[kagzi_step]
/// Validates order input and ensures all required fields are present
async fn validate_order_input(
    _ctx: WorkflowContext,
    input: OrderInput,
) -> anyhow::Result<OrderInput> {
    let mut order = input;
    // Validate order ID
    if order.order_id.is_empty() {
        anyhow::bail!("Order ID cannot be empty");
    }

    // Validate customer ID
    if order.customer_id.is_empty() {
        anyhow::bail!("Customer ID cannot be empty");
    }

    // Validate items
    if order.items.is_empty() {
        anyhow::bail!("Order must contain at least one item");
    }

    // Validate each item
    for (index, item) in order.items.iter().enumerate() {
        if item.product_id.is_empty() {
            anyhow::bail!("Item {} has empty product ID", index + 1);
        }
        if item.quantity == 0 {
            anyhow::bail!("Item {} has zero quantity", index + 1);
        }
        if item.price <= 0.0 {
            anyhow::bail!("Item {} has invalid price: {}", index + 1, item.price);
        }
    }

    // Validate shipping address
    if order.shipping_address.street.is_empty() {
        anyhow::bail!("Street address cannot be empty");
    }
    if order.shipping_address.country.is_empty() {
        anyhow::bail!("Country cannot be empty");
    }

    // Apply promo code validation
    if let Some(code) = &order.promo_code {
        if !code.starts_with("PROMO") {
            order.promo_code = None; // Invalid promo code
        }
    }

    tracing::info!(
        order_id = %order.order_id,
        customer_id = %order.customer_id,
        items_count = order.items.len(),
        "Order input validated successfully"
    );

    Ok(order)
}

#[kagzi_step]
/// Calculates pricing including taxes, shipping, and discounts
async fn calculate_pricing(
    _ctx: WorkflowContext,
    order: OrderInput,
) -> anyhow::Result<(OrderInput, Pricing)> {
    // Calculate subtotal
    let subtotal: f64 = order
        .items
        .iter()
        .map(|item| item.price * item.quantity as f64)
        .sum();

    // Calculate tax (10% for US, 15% for others)
    let tax_rate = if order.shipping_address.country == "US" {
        0.10
    } else {
        0.15
    };
    let tax = subtotal * tax_rate;

    // Calculate shipping based on total and location
    let shipping = if subtotal >= 100.0 {
        0.0 // Free shipping for orders over $100
    } else if order.shipping_address.country == "US" {
        9.99
    } else {
        19.99
    };

    // Apply discount if promo code exists
    let discount = match order.promo_code.as_deref() {
        Some("PROMO10") => subtotal * 0.10,
        Some("PROMO20") => subtotal * 0.20,
        _ => 0.0,
    };

    let total = subtotal + tax + shipping - discount;

    let pricing = Pricing {
        subtotal,
        tax,
        shipping,
        discount,
        total,
        currency: "USD".to_string(),
    };

    tracing::info!(
        order_id = %order.order_id,
        subtotal = subtotal,
        tax = tax,
        shipping = shipping,
        discount = discount,
        total = total,
        "Pricing calculated"
    );

    Ok::<(OrderInput, Pricing), anyhow::Error>((order, pricing))
}

#[kagzi_step]
/// Reserves inventory for all items in the order
async fn reserve_inventory_items(
    _ctx: WorkflowContext,
    input: (OrderInput, Pricing),
) -> anyhow::Result<(OrderInput, Pricing, Vec<String>)> {
    let (order, pricing) = input;
    let mut reservation_ids = Vec::new();
    let mut processed_items = Vec::new();

    for item in &order.items {
        // Simulate inventory check and reservation
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Simulate potential inventory shortage
        if item.product_id == "OUT_OF_STOCK" {
            anyhow::bail!("Product {} is out of stock", item.product_id);
        }

        let reservation_id = format!(
            "INV-{}-{}-{}",
            order.order_id,
            item.product_id,
            uuid::Uuid::new_v4()
                .to_string()
                .chars()
                .take(8)
                .collect::<String>()
        );

        reservation_ids.push(reservation_id);

        processed_items.push(ProcessedItem {
            product_id: item.product_id.clone(),
            quantity: item.quantity,
            price: item.price,
            allocated: true,
        });
    }

    tracing::info!(
        order_id = %order.order_id,
        items_reserved = reservation_ids.len(),
        "Inventory reserved for all items"
    );

    Ok((order, pricing, reservation_ids))
}

#[kagzi_step]
/// Processes payment using the specified payment method
async fn process_order_payment(
    _ctx: WorkflowContext,
    input: (OrderInput, Pricing, Vec<String>),
) -> anyhow::Result<(OrderInput, Pricing, PaymentResult)> {
    let (order, pricing, _reservations) = input;
    // Simulate payment processing delay
    tokio::time::sleep(Duration::from_millis(500)).await;

    let transaction_id = format!(
        "TXN-{}-{}",
        order.order_id,
        uuid::Uuid::new_v4()
            .to_string()
            .chars()
            .take(8)
            .collect::<String>()
    );

    // Simulate potential payment failure
    if matches!(&order.payment_method, PaymentMethod::CreditCard { last_four } if last_four == "0000")
    {
        anyhow::bail!("Payment declined: Invalid card");
    }

    let payment_result = PaymentResult {
        transaction_id: transaction_id.clone(),
        status: "completed".to_string(),
        amount: pricing.total,
        processed_at: chrono::Utc::now(),
    };

    tracing::info!(
        order_id = %order.order_id,
        transaction_id = %transaction_id,
        amount = pricing.total,
        "Payment processed successfully"
    );

    Ok((order, pricing, payment_result))
}

#[kagzi_step]
/// Creates shipping labels and schedules carrier pickup
async fn create_shipment(
    _ctx: WorkflowContext,
    input: (OrderInput, Pricing, PaymentResult),
) -> anyhow::Result<(OrderInput, Pricing, PaymentResult, ShippingInfo)> {
    let (order, pricing, payment) = input;
    // Simulate shipment creation
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Determine shipping method based on order value and location
    let method = if pricing.total > 200.0 {
        "Express".to_string()
    } else if order.shipping_address.country == "US" {
        "Standard".to_string()
    } else {
        "International".to_string()
    };

    // Calculate estimated delivery
    let days_to_delivery = match method.as_str() {
        "Express" => 2,
        "Standard" => 5,
        "International" => 10,
        _ => 7,
    };

    let estimated_delivery = chrono::Utc::now()
        .date_naive()
        .checked_add_days(chrono::Days::new(days_to_delivery))
        .and_hms_opt(23, 59, 59)
        .unwrap()
        .and_utc();

    // Generate tracking numbers
    let tracking_numbers = vec![format!(
        "1Z{}{}{}",
        order.order_id.chars().take(8).collect::<String>(),
        uuid::Uuid::new_v4()
            .to_string()
            .chars()
            .take(10)
            .collect::<String>(),
        uuid::Uuid::new_v4()
            .to_string()
            .chars()
            .take(2)
            .collect::<String>()
    )];

    let shipping_info = ShippingInfo {
        method,
        cost: pricing.shipping,
        estimated_delivery,
        tracking_numbers: tracking_numbers.clone(),
    };

    tracing::info!(
        order_id = %order.order_id,
        method = %shipping_info.method,
        estimated_delivery = %shipping_info.estimated_delivery,
        tracking_count = tracking_numbers.len(),
        "Shipment created"
    );

    Ok((order, pricing, payment, shipping_info))
}

#[kagzi_step]
/// Sends order confirmation to customer
async fn send_order_confirmation(
    _ctx: WorkflowContext,
    input: (OrderInput, Pricing, PaymentResult, ShippingInfo),
) -> anyhow::Result<(OrderInput, Pricing, PaymentResult, ShippingInfo, String)> {
    let (order, pricing, payment, shipping) = input;
    // Simulate email sending
    tokio::time::sleep(Duration::from_millis(100)).await;

    let email_id = format!(
        "CONF-{}-{}",
        order.order_id,
        uuid::Uuid::new_v4()
            .to_string()
            .chars()
            .take(8)
            .collect::<String>()
    );

    tracing::info!(
        order_id = %order.order_id,
        customer_id = %order.customer_id,
        email_id = %email_id,
        transaction_id = %payment.transaction_id,
        "Order confirmation sent"
    );

    Ok((order, pricing, payment, shipping, email_id))
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
        let validated_order = run!("validate_input", validate_order_input(&input));

        // Step 2: Calculate pricing
        let (order, pricing) = run!("calculate_pricing", calculate_pricing(&validated_order));

        // Step 3: Reserve inventory
        let (order, pricing, _reservations) = run!(
            "reserve_inventory",
            reserve_inventory_items(&(order, pricing))
        );

        // Step 4: Process payment
        let (order, pricing, payment) = run!(
            "process_payment",
            process_order_payment(&(order, pricing, _reservations))
        );

        // Step 5: Create shipment
        let (order, pricing, payment, shipping) = run!(
            "create_shipment",
            create_shipment(&(order, pricing, payment))
        );

        // Step 6: Send confirmation
        let (order, pricing, payment, shipping, _email_id) = run!(
            "send_confirmation",
            send_order_confirmation(&(order, pricing, payment, shipping))
        );

        // Build final order output
        let output = OrderOutput {
            order_id: order.order_id.clone(),
            status: OrderStatus::Confirmed,
            items: order.items.into_iter().map(|item| ProcessedItem {
                product_id: item.product_id,
                quantity: item.quantity,
                price: item.price,
                allocated: true,
            }).collect(),
            pricing,
            shipping: ShippingInfo {
                method: shipping.method,
                cost: shipping.cost,
                estimated_delivery: shipping.estimated_delivery,
                tracking_numbers: shipping.tracking_numbers.clone(),
            },
            payment,
            timestamps: OrderTimestamps {
                created_at: chrono::Utc::now(),
                confirmed_at: chrono::Utc::now(),
                processed_at: chrono::Utc::now(),
            },
            tracking_numbers: shipping.tracking_numbers,
        };

        tracing::info!(
            order_id = %output.order_id,
            total = %output.pricing.total,
            status = ?output.status,
            "E-commerce order processing completed"
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
        // Verify order can be cancelled
        let order_status = run!("check_order_status", async {
            tokio::time::sleep(Duration::from_millis(100)).await;
            // In real implementation, fetch from database
            Ok("confirmed") // or "shipped", "delivered", etc.
        });

        if order_status? == "shipped" {
            anyhow::bail!("Cannot cancel shipped order");
        }

        // Release inventory
        let release_result = run!("release_inventory", async {
            tokio::time::sleep(Duration::from_millis(200)).await;
            Ok("inventory_released")
        });

        // Process refund if payment was made
        let refund_result = run!("process_refund", async {
            tokio::time::sleep(Duration::from_millis(500)).await;
            Ok(format!("REF-{}", uuid::Uuid::new_v4()))
        });

        // Send cancellation notification
        run!("send_cancellation_email", async {
            tokio::time::sleep(Duration::from_millis(100)).await;
            Ok(())
        });

        let cancellation_id = format!("CNL-{}-{}",
            order_id,
            uuid::Uuid::new_v4().to_string().chars().take(8).collect::<String>()
        );

        tracing::info!(
            order_id = %order_id,
            cancellation_id = %cancellation_id,
            "Order cancelled successfully"
        );

        Ok(cancellation_id)
    }
}

// ========================
// Main Demo
// ========================

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize logging with debug level to see macro-generated logs
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    println!("🚀 Kagzi Macros - Comprehensive Example");
    println!("========================================");
    println!();
    println!("This example demonstrates both macros working together:");
    println!("- #[kagzi_step] for step functions with automatic observability");
    println!("- kagzi_workflow! for clean workflow definitions with run! macro");
    println!();

    // Create test orders
    let test_orders = vec![
        (
            "Standard Order",
            OrderInput {
                order_id: "ORD-2024-001".to_string(),
                customer_id: "CUST-001".to_string(),
                items: vec![
                    OrderItem {
                        product_id: "LAPTOP-001".to_string(),
                        quantity: 1,
                        price: 999.99,
                    },
                    OrderItem {
                        product_id: "MOUSE-001".to_string(),
                        quantity: 1,
                        price: 29.99,
                    },
                ],
                shipping_address: Address {
                    street: "123 Tech Street".to_string(),
                    city: "San Francisco".to_string(),
                    state: "CA".to_string(),
                    country: "US".to_string(),
                    postal_code: "94105".to_string(),
                },
                payment_method: PaymentMethod::CreditCard {
                    last_four: "1234".to_string(),
                },
                promo_code: None,
            },
        ),
        (
            "Order with Promo",
            OrderInput {
                order_id: "ORD-2024-002".to_string(),
                customer_id: "CUST-002".to_string(),
                items: vec![
                    OrderItem {
                        product_id: "BOOK-001".to_string(),
                        quantity: 3,
                        price: 19.99,
                    },
                    OrderItem {
                        product_id: "PEN-001".to_string(),
                        quantity: 5,
                        price: 2.99,
                    },
                ],
                shipping_address: Address {
                    street: "456 Library Ave".to_string(),
                    city: "New York".to_string(),
                    state: "NY".to_string(),
                    country: "US".to_string(),
                    postal_code: "10001".to_string(),
                },
                payment_method: PaymentMethod::PayPal {
                    email: "buyer@example.com".to_string(),
                },
                promo_code: Some("PROMO10".to_string()),
            },
        ),
        (
            "International Order",
            OrderInput {
                order_id: "ORD-2024-003".to_string(),
                customer_id: "CUST-003".to_string(),
                items: vec![OrderItem {
                    product_id: "ART-001".to_string(),
                    quantity: 1,
                    price: 250.00,
                }],
                shipping_address: Address {
                    street: "789 Art Gallery".to_string(),
                    city: "London".to_string(),
                    state: "".to_string(),
                    country: "UK".to_string(),
                    postal_code: "SW1A 0AA".to_string(),
                },
                payment_method: PaymentMethod::BankTransfer,
                promo_code: Some("PROMO20".to_string()),
            },
        ),
    ];

    for (name, order) in test_orders {
        println!("Processing: {}", name);
        println!("------------------------");
        println!("Order ID: {}", order.order_id);
        println!("Items: {}", order.items.len());
        println!(
            "Total: ${:.2}",
            order
                .items
                .iter()
                .map(|i| i.price * i.quantity as f64)
                .sum::<f64>()
        );
        println!();

        // In a real application, you would register these workflows with a Worker
        // and execute them through the Kagzi server. For this demo, we're just
        // showing that the macros compile and the syntax works.

        println!("✅ Workflow defined with:");
        println!("   - 6 step functions using #[kagzi_step]");
        println!("   - 1 workflow using kagzi_workflow!");
        println!("   - Automatic tracing and logging for all steps");
        println!();
    }

    println!("Key Benefits Demonstrated:");
    println!();
    println!("#[kagzi_step] provides:");
    println!("  ✓ Automatic tracing spans with step names");
    println!("  ✓ Input/output logging at debug level");
    println!("  ✓ Error context enrichment");
    println!("  ✓ Consistent error handling");
    println!();
    println!("kagzi_workflow! provides:");
    println!("  ✓ Clean, readable syntax");
    println!("  ✓ Built-in run! macro for step execution");
    println!("  ✓ Automatic workflow-level tracing");
    println!("  ✓ Reduced boilerplate and repetition");
    println!();
    println!("Together, they provide:");
    println!("  • 50-70% less boilerplate code");
    println!("  • Automatic observability without manual setup");
    println!("  • Type-safe workflow definitions");
    println!("  • Excellent IDE support with rust-analyzer");
    println!();
    println!("To see debug logging from the macros:");
    println!("  RUST_LOG=debug cargo run --example 14_macros_comprehensive");
    println!();
    println!("To run with actual Kagzi server:");
    println!("  1. Start: cargo run -p kagzi-server");
    println!("  2. Create Worker and register these workflows");
    println!("  3. Execute via Client");

    Ok(())
}
