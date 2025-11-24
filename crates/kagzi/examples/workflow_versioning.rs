//! Example: Workflow Versioning
//!
//! This example demonstrates V2 workflow versioning features:
//! - Registering multiple versions of the same workflow
//! - Setting default versions
//! - Running specific workflow versions
//! - Managing version migrations
//!
//! Run with: cargo run --example workflow_versioning

use kagzi::{Kagzi, WorkflowContext};
use serde::{Deserialize, Serialize};
use std::env;
use tracing::info;

#[derive(Debug, Serialize, Deserialize)]
struct OrderInput {
    order_id: String,
    amount: f64,
    customer_email: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct OrderOutput {
    order_id: String,
    status: String,
    version: i32,
    total_amount: f64,
}

/// Version 1 of the order processing workflow - Simple implementation
async fn process_order_v1(
    ctx: WorkflowContext,
    input: OrderInput,
) -> anyhow::Result<OrderOutput> {
    info!("📦 Processing order {} with V1 workflow", input.order_id);

    // V1: Simple validation
    ctx.step("validate-order", async {
        info!("  ✓ Validating order (V1 - basic validation)");
        Ok::<_, anyhow::Error>(())
    })
    .await?;

    // V1: Basic payment processing
    ctx.step("process-payment", async {
        info!("  ✓ Processing payment (V1 - no tax calculation)");
        Ok::<_, anyhow::Error>(())
    })
    .await?;

    // V1: Simple notification
    ctx.step("send-notification", async {
        info!("  ✓ Sending notification (V1 - email only)");
        Ok::<_, anyhow::Error>(())
    })
    .await?;

    Ok(OrderOutput {
        order_id: input.order_id,
        status: "completed".to_string(),
        version: 1,
        total_amount: input.amount,
    })
}

/// Version 2 of the order processing workflow - Enhanced with tax calculation
async fn process_order_v2(
    ctx: WorkflowContext,
    input: OrderInput,
) -> anyhow::Result<OrderOutput> {
    info!("📦 Processing order {} with V2 workflow", input.order_id);

    // V2: Enhanced validation
    ctx.step("validate-order", async {
        info!("  ✓ Validating order (V2 - enhanced validation with fraud check)");
        Ok::<_, anyhow::Error>(())
    })
    .await?;

    // V2: Calculate tax (new feature!)
    let tax_amount: f64 = ctx
        .step("calculate-tax", async {
            info!("  ✓ Calculating tax (V2 - new feature!)");
            let tax_rate = 0.08; // 8% tax
            let tax = input.amount * tax_rate;
            Ok::<_, anyhow::Error>(tax)
        })
        .await?;

    // V2: Process payment with tax
    ctx.step("process-payment", async {
        info!(
            "  ✓ Processing payment (V2 - includes ${:.2} tax)",
            tax_amount
        );
        Ok::<_, anyhow::Error>(())
    })
    .await?;

    // V2: Multi-channel notification
    ctx.step("send-notification", async {
        info!("  ✓ Sending notification (V2 - email + SMS)");
        Ok::<_, anyhow::Error>(())
    })
    .await?;

    Ok(OrderOutput {
        order_id: input.order_id,
        status: "completed".to_string(),
        version: 2,
        total_amount: input.amount + tax_amount,
    })
}

/// Version 3 of the order processing workflow - With loyalty points
async fn process_order_v3(
    ctx: WorkflowContext,
    input: OrderInput,
) -> anyhow::Result<OrderOutput> {
    info!("📦 Processing order {} with V3 workflow", input.order_id);

    // V3: Enhanced validation with ML fraud detection
    ctx.step("validate-order", async {
        info!("  ✓ Validating order (V3 - ML-powered fraud detection)");
        Ok::<_, anyhow::Error>(())
    })
    .await?;

    // V3: Calculate tax
    let tax_amount: f64 = ctx
        .step("calculate-tax", async {
            info!("  ✓ Calculating tax");
            let tax_rate = 0.08;
            let tax = input.amount * tax_rate;
            Ok::<_, anyhow::Error>(tax)
        })
        .await?;

    // V3: Process loyalty points (new feature!)
    let loyalty_points: i32 = ctx
        .step("calculate-loyalty-points", async {
            info!("  ✓ Calculating loyalty points (V3 - new feature!)");
            let points = (input.amount / 10.0) as i32; // 1 point per $10
            Ok::<_, anyhow::Error>(points)
        })
        .await?;

    // V3: Process payment
    ctx.step("process-payment", async {
        info!("  ✓ Processing payment");
        Ok::<_, anyhow::Error>(())
    })
    .await?;

    // V3: Award loyalty points
    ctx.step("award-loyalty-points", async {
        info!("  ✓ Awarded {} loyalty points to customer", loyalty_points);
        Ok::<_, anyhow::Error>(())
    })
    .await?;

    // V3: Multi-channel notification with personalization
    ctx.step("send-notification", async {
        info!("  ✓ Sending personalized notification (V3 - email + SMS + push)");
        Ok::<_, anyhow::Error>(())
    })
    .await?;

    Ok(OrderOutput {
        order_id: input.order_id,
        status: "completed".to_string(),
        version: 3,
        total_amount: input.amount + tax_amount,
    })
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("info,kagzi=debug,sqlx=warn")
        .init();

    let database_url = env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://kagzi:kagzi_dev_password@localhost:5432/kagzi".to_string());

    let kagzi = Kagzi::connect(&database_url).await?;

    info!("🔄 Registering multiple versions of the order processing workflow...\n");

    // Register all three versions
    kagzi
        .register_workflow_version("process-order", 1, process_order_v1)
        .await?;
    info!("✓ Registered process-order v1");

    kagzi
        .register_workflow_version("process-order", 2, process_order_v2)
        .await?;
    info!("✓ Registered process-order v2");

    kagzi
        .register_workflow_version("process-order", 3, process_order_v3)
        .await?;
    info!("✓ Registered process-order v3");

    // Set V3 as the default version for new workflows
    kagzi
        .set_default_workflow_version("process-order", 3)
        .await?;
    info!("✓ Set v3 as default version\n");

    // Start worker
    let worker = kagzi.create_worker();
    tokio::spawn(async move {
        worker.start().await.expect("Worker failed");
    });

    // Demonstration: Run each version explicitly
    info!("=== Demo: Running different versions explicitly ===\n");

    // Run V1 explicitly (legacy workflow)
    info!("1️⃣ Running V1 (legacy workflow):");
    let order1 = OrderInput {
        order_id: "ORDER-001".to_string(),
        amount: 100.0,
        customer_email: "customer1@example.com".to_string(),
    };
    let handle1 = kagzi
        .start_workflow_version("process-order", 1, order1)
        .await?;
    let result1: OrderOutput = serde_json::from_value(handle1.result().await?)?;
    info!(
        "   Result: {} - ${:.2} (version {})\n",
        result1.status, result1.total_amount, result1.version
    );

    // Run V2 explicitly (with tax)
    info!("2️⃣ Running V2 (with tax calculation):");
    let order2 = OrderInput {
        order_id: "ORDER-002".to_string(),
        amount: 100.0,
        customer_email: "customer2@example.com".to_string(),
    };
    let handle2 = kagzi
        .start_workflow_version("process-order", 2, order2)
        .await?;
    let result2: OrderOutput = serde_json::from_value(handle2.result().await?)?;
    info!(
        "   Result: {} - ${:.2} (version {})\n",
        result2.status, result2.total_amount, result2.version
    );

    // Run V3 (default - with loyalty points)
    info!("3️⃣ Running V3 (default version - with loyalty points):");
    let order3 = OrderInput {
        order_id: "ORDER-003".to_string(),
        amount: 100.0,
        customer_email: "customer3@example.com".to_string(),
    };
    // This will use V3 since it's the default
    let handle3 = kagzi.start_workflow("process-order", order3).await?;
    let result3: OrderOutput = serde_json::from_value(handle3.result().await?)?;
    info!(
        "   Result: {} - ${:.2} (version {})\n",
        result3.status, result3.total_amount, result3.version
    );

    info!("=== Summary ===");
    info!("V1 total: ${:.2} (no tax)", result1.total_amount);
    info!("V2 total: ${:.2} (with tax)", result2.total_amount);
    info!("V3 total: ${:.2} (with tax + loyalty)", result3.total_amount);
    info!("\n✅ Workflow versioning example completed!");
    info!("💡 Key takeaway: You can run multiple versions simultaneously");
    info!("   and gradually migrate users to newer versions.");

    Ok(())
}
