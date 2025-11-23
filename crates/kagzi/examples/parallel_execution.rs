//! Parallel execution example
//!
//! Demonstrates the parallel execution capabilities of Kagzi V2:
//! - parallel_vec() for dynamic lists
//! - parallel() for compile-time tuples
//! - race() for first-to-complete semantics
//!
//! Run with: cargo run --example parallel_execution

use kagzi::{Kagzi, WorkflowContext};
use serde::{Deserialize, Serialize};
use std::env;
use std::pin::Pin;
use std::time::Duration;

#[derive(Serialize, Deserialize)]
struct ParallelInput {
    user_ids: Vec<String>,
}

/// Simulate fetching user data (with artificial delay)
async fn fetch_user_data(user_id: String) -> anyhow::Result<String> {
    tokio::time::sleep(Duration::from_millis(100)).await;
    Ok(format!("User data for {}", user_id))
}

/// Simulate fetching posts (with artificial delay)
async fn fetch_posts(user_id: String) -> anyhow::Result<Vec<String>> {
    tokio::time::sleep(Duration::from_millis(150)).await;
    Ok(vec![
        format!("Post 1 by {}", user_id),
        format!("Post 2 by {}", user_id),
    ])
}

/// Simulate fetching comments (with artificial delay)
async fn fetch_comments(user_id: String) -> anyhow::Result<Vec<String>> {
    tokio::time::sleep(Duration::from_millis(120)).await;
    Ok(vec![format!("Comment 1 on {}", user_id)])
}

/// Workflow demonstrating parallel execution
async fn parallel_workflow(
    ctx: WorkflowContext,
    input: ParallelInput,
) -> anyhow::Result<String> {
    println!("\n=== Parallel Execution Workflow ===\n");

    // Example 1: parallel_vec() - Execute dynamic list of steps in parallel
    println!("1. Demonstrating parallel_vec() - Fetching data for {} users", input.user_ids.len());

    let user_steps: Vec<(String, Pin<Box<dyn std::future::Future<Output = anyhow::Result<String>> + Send>>)> = input
        .user_ids
        .clone()
        .into_iter()
        .map(|id| {
            let step_id = format!("fetch-user-{}", id);
            let future: Pin<Box<dyn std::future::Future<Output = anyhow::Result<String>> + Send>> = Box::pin(fetch_user_data(id));
            (step_id, future)
        })
        .collect();

    let start = std::time::Instant::now();
    let user_results = ctx.parallel_vec("fetch-all-users", user_steps).await?;
    let duration = start.elapsed();

    println!("  ✓ Fetched {} users in {:?} (would take ~{}ms sequentially)",
        user_results.len(),
        duration,
        user_results.len() * 100
    );

    // Example 2: parallel() - Execute fixed number of steps with type safety
    println!("\n2. Demonstrating parallel() - Fetching related data for user1");

    let user_id = input.user_ids.first().unwrap_or(&"user1".to_string()).clone();

    let start = std::time::Instant::now();
    let (posts, comments) = ctx
        .parallel(
            "fetch-user-content",
            (
                ("posts", fetch_posts(user_id.clone())),
                ("comments", fetch_comments(user_id.clone())),
            ),
        )
        .await?;
    let duration = start.elapsed();

    println!("  ✓ Fetched posts and comments in {:?}", duration);
    println!("    - Posts: {} items", posts.len());
    println!("    - Comments: {} items", comments.len());

    // Example 3: race() - Return first successful result
    println!("\n3. Demonstrating race() - Racing multiple API endpoints");

    let race_steps: Vec<(String, Pin<Box<dyn std::future::Future<Output = anyhow::Result<String>> + Send>>)> = vec![
        (
            "api-slow".to_string(),
            Box::pin(async {
                tokio::time::sleep(Duration::from_millis(300)).await;
                Ok::<_, anyhow::Error>("Result from slow API".to_string())
            }) as Pin<Box<dyn std::future::Future<Output = anyhow::Result<String>> + Send>>,
        ),
        (
            "api-fast".to_string(),
            Box::pin(async {
                tokio::time::sleep(Duration::from_millis(50)).await;
                Ok::<_, anyhow::Error>("Result from fast API".to_string())
            }) as Pin<Box<dyn std::future::Future<Output = anyhow::Result<String>> + Send>>,
        ),
        (
            "api-medium".to_string(),
            Box::pin(async {
                tokio::time::sleep(Duration::from_millis(150)).await;
                Ok::<_, anyhow::Error>("Result from medium API".to_string())
            }) as Pin<Box<dyn std::future::Future<Output = anyhow::Result<String>> + Send>>,
        ),
    ];

    let start = std::time::Instant::now();
    let (winner, result) = ctx.race("api-race", race_steps).await?;
    let duration = start.elapsed();

    println!("  ✓ Winner: '{}' in {:?}", winner, duration);
    println!("    Result: {}", result);

    println!("\n=== Workflow Complete ===\n");

    Ok(format!(
        "Processed {} users with parallel execution",
        user_results.len()
    ))
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
        .register_workflow("parallel-demo", parallel_workflow)
        .await;

    println!("\nStarting parallel execution workflow...\n");

    let handle = kagzi
        .start_workflow(
            "parallel-demo",
            ParallelInput {
                user_ids: vec![
                    "user1".to_string(),
                    "user2".to_string(),
                    "user3".to_string(),
                    "user4".to_string(),
                    "user5".to_string(),
                ],
            },
        )
        .await?;

    println!("Workflow started: {}\n", handle.run_id());

    // Start worker in same process for this example
    let worker = kagzi.create_worker();

    // Spawn worker in background
    tokio::spawn(async move {
        worker.start().await.expect("Worker failed");
    });

    // Wait for result
    println!("Waiting for workflow to complete...\n");
    let result = handle.result().await?;
    println!("Final result: {}\n", result);

    Ok(())
}
