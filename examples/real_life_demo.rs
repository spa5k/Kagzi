use std::env;
use std::time::Duration;

use kagzi::{Context, Worker};
use serde::{Deserialize, Serialize};
use tokio::time::sleep;

#[path = "common.rs"]
mod common;

// --- Domain Models ---

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Address {
    street: String,
    city: String,
    country: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Item {
    product_id: String,
    quantity: u32,
    price_cents: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Order {
    order_id: String,
    user_id: String,
    items: Vec<Item>,
    shipping_address: Address,
    total_amount_cents: u32,
    is_vip: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PaymentResult {
    transaction_id: String,
    success: bool,
    reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ShipmentDetails {
    order_id: String,
    tracking_number: String,
    carrier: String,
    estimated_delivery: String,
}

// --- Service Steps (Simulated Microservices) ---

/// 1. Fraud Check Service
///    Returns a risk score (0-100). High score = Fraud.
async fn fraud_check_step(order: Order) -> anyhow::Result<u8> {
    println!(
        "🔍 [Fraud Service] Checking order {} for user {}",
        order.order_id, order.user_id
    );

    // Simulate latency (1-2s)
    let delay = rand::random::<u64>() % 1000 + 1000;
    sleep(Duration::from_millis(delay)).await;

    // Business Logic: Large orders from non-VIPs are suspicious
    if !order.is_vip && order.total_amount_cents > 100_000 {
        println!("  -> ⚠️ Suspicious: Large amount for non-VIP");
        return Ok(85); // High risk
    }

    if order.user_id == "fraudster_bot" {
        return Ok(99); // Known bad actor
    }

    println!("  -> ✅ Low risk");
    Ok(10)
}

/// 2. Inventory Service
///    Reserves stock. Returns a reservation ID.
async fn reserve_inventory_step(items: Vec<Item>) -> anyhow::Result<String> {
    println!("📦 [Inventory Service] Reserving {} items...", items.len());

    // Simulate database IO (2-4s)
    let delay = rand::random::<u64>() % 2000 + 2000;
    sleep(Duration::from_millis(delay)).await;

    // Random Chaos: Database connection glitch (Retryable)
    if rand::random::<f32>() < 0.1 {
        println!("  -> 💥 DB Connection Timeout (Simulated)");
        anyhow::bail!("Inventory DB Connection Timeout");
    }

    // Identify out-of-stock item for demo
    for item in &items {
        if item.product_id == "out_of_stock_item" {
            // This is a permanent business error, usually shouldn't retry,
            // but in this simple demo it effectively fails the workflow steps.
            // In a real app we might return a dedicated specific error type.
            println!("  -> ❌ Item {} is out of stock!", item.product_id);
            anyhow::bail!("Item {} Out of Stock", item.product_id);
        }
    }

    let reservation_id = format!("res_{}", uuid::Uuid::new_v4());
    println!("  -> Reserved: {}", reservation_id);
    Ok(reservation_id)
}

/// 3. Payment Gateway
///    Charges the credit card.
async fn charge_payment_step(
    (order_id, amount, _token): (String, u32, String),
) -> anyhow::Result<PaymentResult> {
    println!(
        "💳 [Payment Gateway] Charging {} cents for order {}",
        amount, order_id
    );

    // Simulate 3rd party API URL (4-6s)
    // In a real real-life demo, we might actually hit httpbin.org here,
    // but staying local is faster and more reliable for CI/CD.
    let delay = rand::random::<u64>() % 2000 + 4000;
    sleep(Duration::from_millis(delay)).await;

    // Logic: "broke_user" always fails
    if order_id.contains("broke") {
        println!("  -> ❌ Insufficient Funds");
        return Ok(PaymentResult {
            transaction_id: "".into(),
            success: false,
            reason: Some("Insufficient Funds".into()),
        });
    }

    // Chaos: Network timeout (Retryable)
    if rand::random::<f32>() < 0.1 {
        println!("  -> 💥 Gateway 504 Gateway Timeout (Simulated)");
        anyhow::bail!("Payment Gateway Timeout");
    }

    let tx_id = format!("tx_{}", uuid::Uuid::new_v4());
    println!("  -> ✅ Charged. TX: {}", tx_id);
    Ok(PaymentResult {
        transaction_id: tx_id,
        success: true,
        reason: None,
    })
}

/// 4. Shipping Service
///    Generates a shipping label and tracking number.
async fn ship_package_step(
    (order, reservation_id): (Order, String),
) -> anyhow::Result<ShipmentDetails> {
    println!(
        "🚚 [Shipping Service] Generating label for {} (Res: {})",
        order.order_id, reservation_id
    );

    // Simulating a slower legacy SOAP API (3-5s)
    let delay = rand::random::<u64>() % 2000 + 3000;
    sleep(Duration::from_millis(delay)).await;

    Ok(ShipmentDetails {
        order_id: order.order_id.clone(),
        tracking_number: format!(
            "TRACK-{}-{}",
            order.shipping_address.country,
            uuid::Uuid::new_v4().simple()
        ),
        carrier: "FedEx".to_string(),
        estimated_delivery: "2 Days".to_string(),
    })
}

/// 5. Notification Service
///    Sends confirmation email.
async fn send_email_step(details: ShipmentDetails) -> anyhow::Result<()> {
    println!(
        "📧 [Email Service] Sending confirmation for {}",
        details.order_id
    );
    println!(
        "  Body: Your order is on the way! Tracking: {}",
        details.tracking_number
    );
    Ok(())
}

// --- Workflow ---

async fn order_fulfillment_workflow(mut ctx: Context, order: Order) -> anyhow::Result<String> {
    println!(
        "\n--- 📝 Starting Workflow for Order: {} ---",
        order.order_id
    );

    // Step 1: Fraud Check
    // We treat scores > 80 as failures. This demonstrates flow control based on step output.
    let risk_score = ctx
        .step("fraud_check")
        .run(|| fraud_check_step(order.clone()))
        .await?;

    if risk_score > 80 {
        // We can fail the workflow gracefully or throw an error.
        // Returning Ok("Cancelled") marks it as completed but cancelled from a business logic perspective.
        println!("🚨 Order Rejected due to High Fraud Risk: {}", risk_score);
        return Ok(format!("Order Cancelled: High Fraud Risk ({})", risk_score));
    }

    // Step 2: Reserve Inventory
    // Retries automatically on "DB Connection Timeout"
    let reservation_id = ctx
        .step("reserve_inventory")
        .run(|| reserve_inventory_step(order.items.clone()))
        .await?;

    // Step 3: Charge Payment
    // We assume we have a saved payment token for the user
    let payment_token = "tok_visa_1234".to_string();
    let payment_result = ctx
        .step("charge_payment")
        .run(|| {
            charge_payment_step((
                order.order_id.clone(),
                order.total_amount_cents,
                payment_token,
            ))
        })
        .await?;

    if !payment_result.success {
        // Business logic failure (e.g. Insufficient funds).
        // In a Saga pattern, we would trigger a "release_inventory" step here.
        println!("💸 Payment Failed: {:?}", payment_result.reason);
        // todo: ctx.step("release_inventory")...
        return Ok(format!(
            "Order Cancelled: Payment Failed ({:?})",
            payment_result.reason
        ));
    }

    // Step 4: Ship
    let shipment = ctx
        .step("ship_package")
        .run(|| ship_package_step((order.clone(), reservation_id)))
        .await?;

    // Step 5: Email
    // This is a "fire and forget" style step usually, but we await it here for correctness.
    ctx.step("send_email")
        .run(|| send_email_step(shipment.clone()))
        .await?;

    println!("✅ Order {} Fulfilled Successfully!", order.order_id);
    Ok(format!("Shipped: {}", shipment.tracking_number))
}

// --- Main ---

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let server = env::var("KAGZI_SERVER_URL").unwrap_or_else(|_| "http://localhost:50051".into());
    let namespace = env::var("KAGZI_NAMESPACE").unwrap_or_else(|_| "default".into());

    // 1. Worker Setup
    let server_clone = server.clone();
    let namespace_clone = namespace.clone();
    let _worker_handle = tokio::spawn(async move {
        println!("👷 Worker connecting to {}...", server_clone);
        let mut worker = Worker::new(&server_clone)
            .namespace(&namespace_clone)
            .max_concurrent(2) // Limited to 2 to demonstrate queuing with long-running tasks
            // Retry policy: Exponential backoff for simulated network glitches
            .retry(common::default_retry())
            .workflows([("order_fulfillment", order_fulfillment_workflow)])
            .build()
            .await;

        if let Err(e) = &mut worker {
            eprintln!("Failed to build worker: {:?}", e);
            return;
        }

        let mut worker = worker.unwrap();
        if let Err(e) = worker.run().await {
            eprintln!("Worker error: {:?}", e);
        }
    });

    // 2. Client Setup
    let client = kagzi::Kagzi::connect(&server).await?;

    // 3. Traffic Simulation Loop
    println!("🚀 Starting Infinite Traffic Simulation...");
    println!("   - Press Ctrl+C to stop");
    println!("   - Simulating steady traffic with random BURSTS");
    println!("-----------------------------------");

    let mut rng = rand::rng();
    let mut order_count = 0;

    loop {
        // 5% chance of a traffic spike (DDOS / Viral moment)
        let is_spike = rand::random::<f32>() < 0.05;
        let batch_size = if is_spike {
            let s = rand::Rng::random_range(&mut rng, 50..100);
            println!("\n⚡️ TRAFFIC SPIKE! Incoming batch of {} orders! ⚡️\n", s);
            s
        } else {
            1
        };

        for _ in 0..batch_size {
            order_count += 1;

            // Randomly select scenario type
            let scenario_type = rand::random::<f32>();
            let order = if scenario_type < 0.7 {
                // 70% Happy Path
                Order {
                    order_id: format!("ord_{}_{}", order_count, uuid::Uuid::new_v4().simple()),
                    user_id: "alice".into(),
                    items: vec![Item {
                        product_id: "book".into(),
                        quantity: 1,
                        price_cents: 2500,
                    }],
                    shipping_address: Address {
                        street: "123 Main".into(),
                        city: "Seattle".into(),
                        country: "US".into(),
                    },
                    total_amount_cents: 2500,
                    is_vip: true,
                }
            } else if scenario_type < 0.9 {
                // 20% Fraud (High Value, Non-VIP)
                Order {
                    order_id: format!("ord_{}_{}", order_count, uuid::Uuid::new_v4().simple()),
                    user_id: "sus_user".into(),
                    items: vec![Item {
                        product_id: "diamond".into(),
                        quantity: 5,
                        price_cents: 500_000,
                    }],
                    shipping_address: Address {
                        street: "Dark Alley".into(),
                        city: "Gotham".into(),
                        country: "XX".into(),
                    },
                    total_amount_cents: 2_500_000,
                    is_vip: false,
                }
            } else {
                // 10% Payment Failures
                Order {
                    order_id: format!(
                        "ord_broke_{}_{}",
                        order_count,
                        uuid::Uuid::new_v4().simple()
                    ),
                    user_id: "broke_guy".into(),
                    items: vec![Item {
                        product_id: "ferrari".into(),
                        quantity: 1,
                        price_cents: 999999,
                    }],
                    shipping_address: Address {
                        street: "123".into(),
                        city: "A".into(),
                        country: "US".into(),
                    },
                    total_amount_cents: 999999,
                    is_vip: false,
                }
            };

            client
                .start("order_fulfillment")
                .input(&order)
                .namespace(&namespace)
                .send()
                .await?;
        }

        if !is_spike {
            // Steady state: wait 500ms between orders
            sleep(Duration::from_millis(200)).await;
        } else {
            // After a spike, cool down a bit
            println!("   ... cooling down after spike ...");
            sleep(Duration::from_secs(2)).await;
        }
    }
}
