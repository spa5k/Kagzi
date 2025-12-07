use kagzi::{Worker, WorkflowContext};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::task::JoinHandle;
use tracing::info;
use uuid::Uuid;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let server =
        std::env::var("KAGZI_SERVER_URL").unwrap_or_else(|_| "http://localhost:50051".into());

    let twitter_queue = std::env::var("TWITTER_QUEUE").unwrap_or_else(|_| "queue".into());
    let simple_queue = std::env::var("SIMPLE_QUEUE").unwrap_or_else(|_| "default".into());
    let traced_queue = std::env::var("TRACED_QUEUE").unwrap_or_else(|_| "welcome-queue".into());
    let sleep_failover_queue =
        std::env::var("SLEEP_FAILOVER_QUEUE").unwrap_or_else(|_| "sleep-failover".into());
    let observability_queue =
        std::env::var("OBSERVABILITY_QUEUE").unwrap_or_else(|_| "observability-test-queue".into());
    let comprehensive_queue =
        std::env::var("COMPREHENSIVE_QUEUE").unwrap_or_else(|_| "comprehensive-test-queue".into());

    let mut handles: Vec<JoinHandle<anyhow::Result<()>>> = Vec::new();

    handles.push(
        spawn_worker(server.clone(), twitter_queue, |worker| {
            worker.register("UserSignup", twitter_example::user_signup);
        })
        .await?,
    );

    handles.push(
        spawn_worker(server.clone(), simple_queue, |worker| {
            worker.register("my_workflow", simple_example::my_workflow);
        })
        .await?,
    );

    handles.push(
        spawn_worker(server.clone(), traced_queue, |worker| {
            worker.register("WelcomeEmail", traced_workflow::welcome_workflow);
        })
        .await?,
    );

    handles.push(
        spawn_worker(server.clone(), sleep_failover_queue, |worker| {
            worker.register("Sleep15Seconds", sleep_failover::sleep_workflow);
        })
        .await?,
    );

    handles.push(
        spawn_worker(server.clone(), observability_queue, |worker| {
            worker.register(
                "UserOnboarding",
                observability_example::user_onboarding_workflow,
            );
        })
        .await?,
    );

    handles.push(
        spawn_worker(server.clone(), comprehensive_queue, |worker| {
            worker.register(
                "OrderProcessing",
                comprehensive_example::order_processing_workflow,
            );
            worker.register("SleepWorkflow", comprehensive_example::sleep_workflow);
            worker.register("FailingWorkflow", comprehensive_example::failing_workflow);
            worker.register("SimpleWorkflow", comprehensive_example::simple_workflow);
            worker.register(
                "DataPipeline",
                comprehensive_example::data_pipeline_workflow,
            );
        })
        .await?,
    );

    info!("Worker-only example started; awaiting workers. Press Ctrl+C to exit.");

    for handle in handles {
        handle.await??;
    }

    Ok(())
}

async fn spawn_worker<F>(
    server: String,
    queue: String,
    register: F,
) -> anyhow::Result<JoinHandle<anyhow::Result<()>>>
where
    F: FnOnce(&mut Worker) + Send + 'static,
{
    let mut worker = Worker::builder(&server, &queue).build().await?;
    register(&mut worker);

    Ok(tokio::spawn(async move { worker.run().await }))
}

mod twitter_example {
    use super::*;

    #[derive(Debug, Serialize, Deserialize)]
    pub struct Input {
        email: String,
    }

    #[derive(Debug, Serialize, Deserialize)]
    pub struct Output {
        user_id: String,
        status: String,
    }

    #[derive(Debug, Serialize, Deserialize, Clone)]
    pub struct User {
        id: String,
        email: String,
    }

    pub async fn user_signup(mut ctx: WorkflowContext, input: Input) -> anyhow::Result<Output> {
        info!("Starting user signup workflow for: {}", input.email);

        let user = ctx.run("create_user", create_user(input.email)).await?;

        ctx.run("send_welcome_email", send_welcome_email(user.clone()))
            .await?;

        info!("Sleeping for 10 seconds...");
        ctx.sleep(Duration::from_secs(10)).await?;
        info!("Resumed after sleep");

        ctx.run("send_review_email", send_review_email(user.clone()))
            .await?;

        info!("Workflow completed successfully!");
        Ok(Output {
            user_id: user.id,
            status: "onboarded".to_string(),
        })
    }

    async fn create_user(email: String) -> anyhow::Result<User> {
        info!("Creating user with email: {}", email);
        let user = User {
            id: Uuid::now_v7().to_string(),
            email,
        };
        info!("Created user: {}", user.id);
        Ok(user)
    }

    async fn send_welcome_email(user: User) -> anyhow::Result<()> {
        info!("Sending welcome email to user: {}", user.id);
        Ok(())
    }

    async fn send_review_email(user: User) -> anyhow::Result<()> {
        if !user.email.contains('@') {
            return Err(anyhow::anyhow!("Invalid email"));
        }
        info!("Sending review email to user: {}", user.id);
        Ok(())
    }
}

mod simple_example {
    use super::*;

    #[derive(Serialize, Deserialize, Debug)]
    pub struct MyInput {
        name: String,
    }

    #[derive(Serialize, Deserialize, Debug)]
    pub struct MyOutput {
        message: String,
    }

    async fn step1() -> anyhow::Result<String> {
        info!("Running step 1");
        Ok("Hello".to_string())
    }

    async fn step2() -> anyhow::Result<String> {
        info!("Running step 2");
        Ok("World".to_string())
    }

    pub async fn my_workflow(mut ctx: WorkflowContext, input: MyInput) -> anyhow::Result<MyOutput> {
        info!("Workflow started with input: {:?}", input);

        let step1_res = ctx.run("step1", step1()).await?;

        info!("Step 1 result: {}", step1_res);
        ctx.sleep(Duration::from_secs(2)).await?;
        info!("Woke up from sleep");

        let step2_res = ctx.run("step2", step2()).await?;

        Ok(MyOutput {
            message: format!("{} {}, {}", step1_res, step2_res, input.name),
        })
    }
}

mod traced_workflow {
    use super::*;

    #[derive(Debug, Serialize, Deserialize)]
    pub struct Input {
        message: String,
    }

    #[derive(Debug, Serialize, Deserialize)]
    pub struct Output {
        response: String,
        processed_at: String,
    }

    async fn process_message(message: String) -> anyhow::Result<String> {
        Ok(format!("Hello, {}! Your message was processed.", message))
    }

    async fn add_timestamp(response: String) -> anyhow::Result<Output> {
        Ok(Output {
            response,
            processed_at: chrono::Utc::now().to_rfc3339(),
        })
    }

    pub async fn welcome_workflow(
        mut ctx: WorkflowContext,
        input: Input,
    ) -> anyhow::Result<Output> {
        let processed = ctx
            .run("process_message", process_message(input.message))
            .await?;

        let output = ctx.run("add_timestamp", add_timestamp(processed)).await?;

        Ok(output)
    }
}

mod sleep_failover {
    use super::*;

    pub async fn sleep_workflow(
        _ctx: WorkflowContext,
        input: serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        let host = hostname::get()
            .ok()
            .and_then(|h| h.into_string().ok())
            .unwrap_or_else(|| "unknown-host".to_string());
        let pid = std::process::id();

        info!(host = %host, pid = pid, input = ?input, "Sleep workflow started");
        tokio::time::sleep(Duration::from_secs(15)).await;
        info!(host = %host, pid = pid, "Sleep workflow finished");

        Ok(serde_json::json!({
            "handled_by": host,
            "pid": pid,
            "slept_seconds": 15
        }))
    }
}

mod observability_example {
    use super::*;

    #[derive(Debug, Serialize, Deserialize)]
    pub struct UserData {
        user_id: String,
        email: String,
        plan: String,
    }

    #[derive(Debug, Serialize, Deserialize)]
    struct FetchUserInput {
        user_id: String,
    }

    #[derive(Debug, Serialize, Deserialize)]
    struct SendEmailInput {
        to: String,
        subject: String,
        template: String,
    }

    #[derive(Debug, Serialize, Deserialize)]
    struct EmailResult {
        message_id: String,
        sent_at: String,
    }

    async fn fetch_user(input: &FetchUserInput) -> anyhow::Result<UserData> {
        tokio::time::sleep(Duration::from_millis(100)).await;
        Ok(UserData {
            user_id: input.user_id.clone(),
            email: format!("{}@example.com", input.user_id),
            plan: "premium".to_string(),
        })
    }

    async fn send_email(input: &SendEmailInput) -> anyhow::Result<EmailResult> {
        info!(
            "Sending email to {} with template {}",
            input.to, input.template
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
        Ok(EmailResult {
            message_id: Uuid::now_v7().to_string(),
            sent_at: chrono::Utc::now().to_rfc3339(),
        })
    }

    pub async fn user_onboarding_workflow(
        mut ctx: WorkflowContext,
        input: UserData,
    ) -> anyhow::Result<super::observability_example::WorkflowOutput> {
        let fetch_input = FetchUserInput {
            user_id: input.user_id.clone(),
        };
        let user = ctx
            .run_with_input("fetch_user_data", &fetch_input, fetch_user(&fetch_input))
            .await?;

        let welcome_input = SendEmailInput {
            to: user.email.clone(),
            subject: "Welcome to our platform!".to_string(),
            template: "welcome_v2".to_string(),
        };
        ctx.run_with_input(
            "send_welcome_email",
            &welcome_input,
            send_email(&welcome_input),
        )
        .await?;

        ctx.sleep(Duration::from_secs(2)).await?;

        let tips_input = SendEmailInput {
            to: user.email.clone(),
            subject: "Tips for getting started".to_string(),
            template: "tips_v1".to_string(),
        };
        ctx.run_with_input("send_tips_email", &tips_input, send_email(&tips_input))
            .await?;

        Ok(WorkflowOutput {
            user_id: user.user_id,
            emails_sent: 2,
            final_status: "onboarded".to_string(),
        })
    }

    #[derive(Debug, Serialize, Deserialize)]
    pub struct WorkflowOutput {
        user_id: String,
        emails_sent: u32,
        final_status: String,
    }
}

mod comprehensive_example {
    use super::*;

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct OrderInput {
        order_id: String,
        customer_email: String,
        items: Vec<OrderItem>,
        total_amount: f64,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct OrderItem {
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
    pub struct OrderWorkflowOutput {
        order_id: String,
        status: String,
        transaction_id: Option<String>,
        notifications_sent: u32,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct FailingWorkflowInput {
        should_fail: bool,
        fail_message: String,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct SimpleInput {
        message: String,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct SimpleOutput {
        result: String,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct DataPipelineInput {
        raw_data: String,
        multiplier: u32,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
    struct ParsedData {
        numbers: Vec<i32>,
        source: String,
        parsed_at: String,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct TransformInput {
        parsed: ParsedData,
        multiplier: u32,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
    struct TransformedData {
        values: Vec<i32>,
        sum: i32,
        count: usize,
        multiplier_used: u32,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct AggregateInput {
        transformed: TransformedData,
        include_stats: bool,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
    struct AggregatedResult {
        total: i32,
        average: f64,
        min: i32,
        max: i32,
        item_count: usize,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct DataPipelineOutput {
        original_input: String,
        parsed_count: usize,
        transformed_sum: i32,
        final_result: AggregatedResult,
        pipeline_verification: PipelineVerification,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct PipelineVerification {
        step1_output_count: usize,
        step2_received_count: usize,
        step2_output_sum: i32,
        step3_received_sum: i32,
        all_values_match: bool,
    }

    async fn validate_order(input: &ValidateOrderInput) -> anyhow::Result<ValidateOrderOutput> {
        tokio::time::sleep(Duration::from_millis(50)).await;
        Ok(ValidateOrderOutput {
            is_valid: true,
            validated_items: input.items.len() as u32,
            validation_timestamp: chrono::Utc::now().to_rfc3339(),
        })
    }

    async fn process_payment(input: &ProcessPaymentInput) -> anyhow::Result<ProcessPaymentOutput> {
        info!(
            "Processing payment for order {} amount {}",
            input.order_id, input.amount
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
        Ok(ProcessPaymentOutput {
            transaction_id: format!("txn_{}", &Uuid::now_v7().to_string()[..8]),
            status: "SUCCESS".to_string(),
            processed_at: chrono::Utc::now().to_rfc3339(),
        })
    }

    async fn send_notification(
        input: &SendNotificationInput,
    ) -> anyhow::Result<SendNotificationOutput> {
        info!("Sending notification {} to {}", input.template, input.to);
        tokio::time::sleep(Duration::from_millis(30)).await;
        Ok(SendNotificationOutput {
            message_id: format!("msg_{}", &Uuid::now_v7().to_string()[..8]),
            sent_at: chrono::Utc::now().to_rfc3339(),
        })
    }

    async fn parse_data(raw: &str) -> anyhow::Result<ParsedData> {
        tokio::time::sleep(Duration::from_millis(50)).await;
        let numbers: Vec<i32> = raw
            .split(',')
            .filter_map(|s| s.trim().parse().ok())
            .collect();
        Ok(ParsedData {
            numbers,
            source: raw.to_string(),
            parsed_at: chrono::Utc::now().to_rfc3339(),
        })
    }

    async fn transform_data(input: &TransformInput) -> anyhow::Result<TransformedData> {
        tokio::time::sleep(Duration::from_millis(50)).await;
        let values: Vec<i32> = input
            .parsed
            .numbers
            .iter()
            .map(|n| n * input.multiplier as i32)
            .collect();
        let sum: i32 = values.iter().sum();
        let count = values.len();
        Ok(TransformedData {
            values,
            sum,
            count,
            multiplier_used: input.multiplier,
        })
    }

    async fn aggregate_data(input: &AggregateInput) -> anyhow::Result<AggregatedResult> {
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

        Ok(AggregatedResult {
            total,
            average,
            min,
            max,
            item_count: values.len(),
        })
    }

    pub async fn order_processing_workflow(
        mut ctx: WorkflowContext,
        input: OrderInput,
    ) -> anyhow::Result<OrderWorkflowOutput> {
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
        ctx.run_with_input(
            "send_confirmation_email",
            &email_input,
            send_notification(&email_input),
        )
        .await?;

        Ok(OrderWorkflowOutput {
            order_id: input.order_id,
            status: "COMPLETED".to_string(),
            transaction_id: Some(payment.transaction_id),
            notifications_sent: 1,
        })
    }

    pub async fn sleep_workflow(
        mut ctx: WorkflowContext,
        input: SimpleInput,
    ) -> anyhow::Result<SimpleOutput> {
        ctx.run("pre_sleep_step", async {
            tokio::time::sleep(Duration::from_millis(50)).await;
            Ok::<_, anyhow::Error>("pre_sleep_done".to_string())
        })
        .await?;

        ctx.sleep(Duration::from_secs(1)).await?;

        ctx.run("post_sleep_step", async {
            tokio::time::sleep(Duration::from_millis(50)).await;
            Ok::<_, anyhow::Error>("post_sleep_done".to_string())
        })
        .await?;

        Ok(SimpleOutput {
            result: format!("Processed: {}", input.message),
        })
    }

    pub async fn failing_workflow(
        mut ctx: WorkflowContext,
        input: FailingWorkflowInput,
    ) -> anyhow::Result<SimpleOutput> {
        ctx.run("successful_step", async {
            Ok::<_, anyhow::Error>("success".to_string())
        })
        .await?;

        if input.should_fail {
            anyhow::bail!("{}", input.fail_message);
        }

        Ok(SimpleOutput {
            result: "Should not reach here if failing".to_string(),
        })
    }

    pub async fn simple_workflow(
        _ctx: WorkflowContext,
        input: SimpleInput,
    ) -> anyhow::Result<SimpleOutput> {
        tokio::time::sleep(Duration::from_millis(100)).await;

        Ok(SimpleOutput {
            result: format!("Processed: {}", input.message),
        })
    }

    pub async fn data_pipeline_workflow(
        mut ctx: WorkflowContext,
        input: DataPipelineInput,
    ) -> anyhow::Result<DataPipelineOutput> {
        let step1_output = ctx
            .run_with_input(
                "parse_raw_data",
                &input.raw_data,
                parse_data(&input.raw_data),
            )
            .await?;

        let step1_output_count = step1_output.numbers.len();

        let transform_input = TransformInput {
            parsed: step1_output.clone(),
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

        let aggregate_input = AggregateInput {
            transformed: step2_output.clone(),
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

        let verification = PipelineVerification {
            step1_output_count,
            step2_received_count,
            step2_output_sum,
            step3_received_sum,
            all_values_match: step2_received_count == step1_output_count
                && step3_received_sum == step2_output_sum,
        };

        Ok(DataPipelineOutput {
            original_input: input.raw_data,
            parsed_count: step1_output_count,
            transformed_sum: step2_output_sum,
            final_result: step3_output,
            pipeline_verification: verification,
        })
    }
}
