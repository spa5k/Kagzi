# Migration Guide: V1 to V2

This guide helps you upgrade your Kagzi workflows from V1 to V2, taking advantage of the new production-ready features.

## Overview of V2 Changes

V2 introduces several powerful new features while maintaining full backward compatibility:

- ✅ **Retry Policies**: Automatic retry with exponential backoff
- ✅ **Parallel Execution**: Execute steps concurrently with advanced error handling
- ✅ **Workflow Versioning**: Manage multiple workflow versions simultaneously
- ✅ **Production Workers**: Enhanced worker management with health monitoring
- ✅ **Health Monitoring**: System observability and worker health checks

## Backward Compatibility

**Good news**: All V1 code continues to work without changes! V2 is fully backward compatible.

You can adopt V2 features gradually without breaking existing workflows.

## Migration Steps

### 1. Database Migrations (Automatic)

V2 includes automatic database migrations. When you connect, migrations run automatically:

```rust
// V1 and V2 - Same API
let kagzi = Kagzi::connect("postgres://localhost/kagzi").await?;
// Migrations run automatically ✓
```

### 2. Upgrading to Retry Policies

**Before (V1): Manual retry logic**
```rust
// V1: Manual retry with custom logic
ctx.step("api-call", async {
    let mut attempts = 0;
    let max_attempts = 3;

    loop {
        attempts += 1;
        match make_api_call().await {
            Ok(result) => return Ok(result),
            Err(e) if attempts < max_attempts => {
                tokio::time::sleep(Duration::from_secs(2_u64.pow(attempts))).await;
                continue;
            }
            Err(e) => return Err(e),
        }
    }
}).await?;
```

**After (V2): Declarative retry policies**
```rust
use kagzi::{StepBuilder, RetryPolicy, RetryPredicate};

// V2: Automatic retry with exponential backoff
StepBuilder::new("api-call")
    .retry_policy(RetryPolicy::exponential())
    .retry_predicate(RetryPredicate::OnRetryableError)
    .execute(&ctx, async {
        make_api_call().await
    })
    .await?;
```

**Benefits**:
- Cleaner code - retry logic is declarative
- Durable retries - survives worker restarts
- Jitter support to prevent thundering herd
- Configurable error classification

### 3. Upgrading to Parallel Execution

**Before (V1): Manual parallel execution**
```rust
// V1: Manual parallel execution with tokio::join
let (user, posts, comments) = tokio::join!(
    ctx.step("fetch-user", fetch_user()),
    ctx.step("fetch-posts", fetch_posts()),
    ctx.step("fetch-comments", fetch_comments()),
);
```

**After (V2): Type-safe parallel execution with memoization**
```rust
use kagzi::ParallelExecutor;

// V2: Enhanced parallel execution with ParallelExecutor
let parallel_group = uuid::Uuid::new_v4();
let executor = ParallelExecutor::new(
    &ctx,
    parallel_group,
    None,
    ParallelErrorStrategy::FailFast, // or CollectAll
);

// Execute with full memoization support
let handles = vec![
    tokio::spawn(executor.execute_step("fetch-user", fetch_user())),
    tokio::spawn(executor.execute_step("fetch-posts", fetch_posts())),
    tokio::spawn(executor.execute_step("fetch-comments", fetch_comments())),
];

let results = futures::future::join_all(handles).await;
```

**Benefits**:
- Full memoization - cached results survive restarts
- Error strategies - FailFast or CollectAll
- Parallel tracking - monitor parallel execution progress
- Type-safe APIs

**Alternative: Use convenience methods**
```rust
// V2: Tuple-based parallel (for fixed number of steps)
let (user, posts) = ctx.parallel(
    "user-data",
    (
        ("user", fetch_user()),
        ("posts", fetch_posts()),
    )
).await?;

// V2: Vec-based parallel (for dynamic lists)
let results = ctx.parallel_vec("fetch-all", vec![
    ("item-1".to_string(), Box::pin(fetch_item_1())),
    ("item-2".to_string(), Box::pin(fetch_item_2())),
]).await?;

// V2: Race execution (first to complete wins)
let (winner, result) = ctx.race("fastest-api", vec![
    ("api-1".to_string(), Box::pin(call_api_1())),
    ("api-2".to_string(), Box::pin(call_api_2())),
]).await?;
```

### 4. Adopting Workflow Versioning

**Before (V1): Single workflow version**
```rust
// V1: Only one version can exist
kagzi.register_workflow("process-order", process_order).await;

// Starting a workflow
let handle = kagzi.start_workflow("process-order", input).await?;
```

**After (V2): Multi-version support**
```rust
// V2: Register multiple versions
kagzi.register_workflow_version("process-order", 1, process_order_v1).await?;
kagzi.register_workflow_version("process-order", 2, process_order_v2).await?;
kagzi.register_workflow_version("process-order", 3, process_order_v3).await?;

// Set default version for new workflows
kagzi.set_default_workflow_version("process-order", 3).await?;

// Start with specific version
let handle = kagzi.start_workflow_version("process-order", 2, input).await?;

// Or use default version
let handle = kagzi.start_workflow("process-order", input).await?;
```

**Benefits**:
- Multiple versions coexist
- Gradual rollout of changes
- Canary deployments
- Easy rollback to previous versions
- Long-running workflows can complete on old versions

**Migration strategy**:
1. Register your existing workflow as version 1
2. Develop and test version 2 alongside version 1
3. Set version 2 as default when ready
4. Both versions run side-by-side until old workflows complete

### 5. Upgrading Worker Configuration

**Before (V1): Basic worker creation**
```rust
// V1: Simple worker with defaults
let worker = kagzi.create_worker();
worker.start().await?;
```

**After (V2): Production-ready workers with custom configuration**
```rust
use kagzi::WorkerBuilder;

// V2: Fully configured production worker
let worker = kagzi
    .create_worker_builder()
    .worker_id(format!("worker-{}", std::process::id()))
    .max_concurrent_workflows(10)         // Limit concurrent execution
    .heartbeat_interval_secs(30)          // Health monitoring
    .poll_interval_ms(500)                // Polling frequency
    .graceful_shutdown_timeout_secs(300)  // Wait for workflows on shutdown
    .hostname(gethostname::gethostname().to_string_lossy().to_string())
    .build();

// Worker handles SIGTERM/SIGINT gracefully
worker.start().await?;
```

**Benefits**:
- Graceful shutdown - wait for workflows to complete
- Heartbeat monitoring - detect unhealthy workers
- Concurrent workflow limiting - prevent overload
- Worker registration - track all workers in database
- Configurable polling - optimize for your workload

**Backward compatibility**: `kagzi.create_worker()` still works with sensible defaults.

### 6. Adding Health Monitoring

**New in V2**: Comprehensive health monitoring

```rust
use kagzi::{HealthChecker, HealthCheckConfig};

// Configure health thresholds
let config = HealthCheckConfig {
    heartbeat_timeout_secs: 120,  // Unhealthy after 2 minutes
    heartbeat_warning_secs: 60,   // Warning after 1 minute
};

let health_checker = HealthChecker::with_config(kagzi.db_handle(), config);

// Check database health
health_checker.check_database_health().await?;

// Monitor worker health
let workers = kagzi_core::queries::get_active_workers(kagzi.db_handle().pool()).await?;
for worker in workers {
    let health = health_checker.check_worker_health(&worker, 0).await;

    match health.status {
        HealthStatus::Healthy => println!("✓ {} is healthy", worker.worker_name),
        HealthStatus::Degraded => println!("⚠ {} is degraded", worker.worker_name),
        HealthStatus::Unhealthy => println!("✗ {} is unhealthy", worker.worker_name),
    }
}

// Comprehensive system health
let result = health_checker.comprehensive_check().await;
if !result.is_healthy() {
    eprintln!("System health issues detected!");
}
```

**Benefits**:
- Proactive monitoring
- Early detection of issues
- Integration with monitoring systems
- Worker lifecycle management

## Complete Migration Example

Here's a complete example showing V1 vs V2:

### V1 Code
```rust
use kagzi::{Kagzi, WorkflowContext};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
struct OrderInput {
    order_id: String,
    amount: f64,
}

async fn process_order_v1(
    ctx: WorkflowContext,
    input: OrderInput,
) -> anyhow::Result<String> {
    // Manual retry logic
    let mut attempts = 0;
    let user = loop {
        attempts += 1;
        match ctx.step("fetch-user", fetch_user()).await {
            Ok(u) => break u,
            Err(e) if attempts < 3 => {
                tokio::time::sleep(Duration::from_secs(2)).await;
                continue;
            }
            Err(e) => return Err(e),
        }
    };

    // Sequential execution
    let payment = ctx.step("process-payment", process_payment(&input)).await?;
    let inventory = ctx.step("update-inventory", update_inventory(&input)).await?;

    // Simple notification
    ctx.step("notify", send_email(&input)).await?;

    Ok("completed".to_string())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let kagzi = Kagzi::connect("postgres://localhost/kagzi").await?;
    kagzi.register_workflow("process-order", process_order_v1).await;

    let worker = kagzi.create_worker();
    worker.start().await?;

    Ok(())
}
```

### V2 Code (Enhanced)
```rust
use kagzi::{
    Kagzi, WorkflowContext, StepBuilder, RetryPolicy,
    RetryPredicate, ParallelExecutor, ParallelErrorStrategy,
    WorkerBuilder, HealthChecker,
};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
struct OrderInput {
    order_id: String,
    amount: f64,
}

async fn process_order_v2(
    ctx: WorkflowContext,
    input: OrderInput,
) -> anyhow::Result<String> {
    // Automatic retry with exponential backoff
    let user = StepBuilder::new("fetch-user")
        .retry_policy(RetryPolicy::exponential())
        .retry_predicate(RetryPredicate::OnRetryableError)
        .execute(&ctx, fetch_user())
        .await?;

    // Parallel execution with memoization
    let parallel_group = uuid::Uuid::new_v4();
    let executor = ParallelExecutor::new(
        &ctx,
        parallel_group,
        None,
        ParallelErrorStrategy::FailFast,
    );

    let (payment, inventory) = tokio::try_join!(
        executor.execute_step("process-payment", process_payment(&input)),
        executor.execute_step("update-inventory", update_inventory(&input)),
    )?;

    // Multi-channel notification
    ctx.parallel_vec("notify", vec![
        ("email".to_string(), Box::pin(send_email(&input))),
        ("sms".to_string(), Box::pin(send_sms(&input))),
    ]).await?;

    Ok("completed".to_string())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let kagzi = Kagzi::connect("postgres://localhost/kagzi").await?;

    // Register with versioning
    kagzi.register_workflow_version("process-order", 2, process_order_v2).await?;
    kagzi.set_default_workflow_version("process-order", 2).await?;

    // Production-ready worker
    let worker = kagzi
        .create_worker_builder()
        .worker_id("order-worker-1".to_string())
        .max_concurrent_workflows(10)
        .heartbeat_interval_secs(30)
        .graceful_shutdown_timeout_secs(300)
        .build();

    // Health monitoring
    let health_checker = HealthChecker::new(kagzi.db_handle());
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        loop {
            interval.tick().await;
            if let Ok(result) = health_checker.comprehensive_check().await {
                if !result.is_healthy() {
                    eprintln!("Health check failed: {:?}", result.errors);
                }
            }
        }
    });

    worker.start().await?;

    Ok(())
}
```

## Step-by-Step Migration Plan

### Phase 1: Update Dependencies (Day 1)
1. Update to Kagzi V2 in `Cargo.toml`
2. Run `cargo build` - all V1 code still works
3. Test existing workflows

### Phase 2: Add Retry Policies (Week 1)
1. Identify steps that need retries
2. Replace manual retry logic with `StepBuilder`
3. Configure appropriate retry policies
4. Test retry behavior

### Phase 3: Introduce Parallel Execution (Week 2)
1. Identify steps that can run in parallel
2. Refactor using `ParallelExecutor` or convenience methods
3. Test error handling strategies
4. Monitor performance improvements

### Phase 4: Enable Workflow Versioning (Week 3)
1. Register existing workflows as version 1
2. Develop and test new versions
3. Set up gradual rollout strategy
4. Monitor both versions in production

### Phase 5: Upgrade Workers (Week 4)
1. Configure production worker settings
2. Set up graceful shutdown
3. Implement health monitoring
4. Deploy with rolling updates

### Phase 6: Add Observability (Week 5)
1. Set up health check endpoints
2. Integrate with monitoring systems
3. Configure alerting thresholds
4. Create operational dashboards

## Common Migration Patterns

### Pattern 1: Adding Retries to Existing Steps

```rust
// Find this pattern in your V1 code:
ctx.step("api-call", make_api_call()).await?

// Replace with:
StepBuilder::new("api-call")
    .retry_policy(RetryPolicy::exponential())
    .execute(&ctx, make_api_call())
    .await?
```

### Pattern 2: Parallelizing Sequential Steps

```rust
// V1: Sequential
let a = ctx.step("step-a", step_a()).await?;
let b = ctx.step("step-b", step_b()).await?;
let c = ctx.step("step-c", step_c()).await?;

// V2: Parallel (if steps are independent)
let (a, b, c) = ctx.parallel(
    "parallel-group",
    (
        ("step-a", step_a()),
        ("step-b", step_b()),
        ("step-c", step_c()),
    )
).await?;
```

### Pattern 3: Converting to Versioned Workflows

```rust
// V1: Single registration
kagzi.register_workflow("my-workflow", my_workflow).await;

// V2: Versioned registration
kagzi.register_workflow_version("my-workflow", 1, my_workflow_v1).await?;
kagzi.register_workflow_version("my-workflow", 2, my_workflow_v2).await?;
kagzi.set_default_workflow_version("my-workflow", 2).await?;
```

## Troubleshooting

### Issue: Retry policy not working
**Cause**: Step errors aren't classified as retryable
**Solution**: Ensure errors are `ErrorKind::NetworkError`, `Timeout`, or `DatabaseError`

### Issue: Parallel steps not executing concurrently
**Cause**: Missing `tokio::spawn`
**Solution**: Wrap executor calls in `tokio::spawn` or use convenience methods

### Issue: Worker not shutting down gracefully
**Cause**: Workflows taking too long
**Solution**: Increase `graceful_shutdown_timeout_secs` or optimize workflows

### Issue: Health checks always showing unhealthy
**Cause**: Heartbeat interval too short
**Solution**: Adjust `HealthCheckConfig` thresholds

## Testing Your Migration

Run this checklist after migrating:

- [ ] All existing V1 workflows still execute
- [ ] Retry policies work as expected
- [ ] Parallel execution provides performance benefits
- [ ] Multiple workflow versions coexist
- [ ] Workers shut down gracefully
- [ ] Health checks report accurate status
- [ ] Database migrations completed successfully
- [ ] No breaking changes in API usage

## Need Help?

- Check the examples in `examples/` directory
- Review API documentation: `cargo doc --open`
- File issues: https://github.com/yourusername/kagzi/issues
- Refer to architecture docs: `docs/ARCHITECTURE.md`

## Summary

V2 brings production-ready features while maintaining full backward compatibility:
- ✅ No breaking changes
- ✅ Incremental adoption
- ✅ Better performance with parallel execution
- ✅ More reliable with retry policies
- ✅ Production-ready with health monitoring
- ✅ Future-proof with workflow versioning

Start small, migrate step by step, and enjoy the benefits of V2!
