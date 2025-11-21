use kagzi::{Kagzi, WorkflowContext};
use serde::{Deserialize, Serialize};
use std::env;

#[derive(Debug, Serialize, Deserialize, Clone)]
struct TestInput {
    value: String,
}

async fn simple_workflow(_ctx: WorkflowContext, input: TestInput) -> anyhow::Result<String> {
    Ok(format!("Processed: {}", input.value))
}

async fn workflow_with_step(ctx: WorkflowContext, input: TestInput) -> anyhow::Result<String> {
    let result = ctx
        .step("process-step", async {
            Ok::<_, anyhow::Error>(format!("Step result: {}", input.value))
        })
        .await?;

    Ok(result)
}

fn get_test_database_url() -> String {
    env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5432/kagzi_test".to_string())
}

async fn setup_test_db() -> anyhow::Result<Kagzi> {
    let database_url = get_test_database_url();
    Kagzi::connect(&database_url).await
}

#[tokio::test]
#[ignore] // Requires database
async fn test_connect_to_database() {
    let result = setup_test_db().await;
    assert!(result.is_ok());
}

#[tokio::test]
#[ignore] // Requires database
async fn test_register_workflow() {
    let kagzi = setup_test_db().await.unwrap();
    kagzi
        .register_workflow("test-workflow", simple_workflow)
        .await;
    // If we get here without panicking, registration succeeded
}

#[tokio::test]
#[ignore] // Requires database
async fn test_start_workflow() {
    let kagzi = setup_test_db().await.unwrap();
    kagzi
        .register_workflow("test-workflow", simple_workflow)
        .await;

    let input = TestInput {
        value: "test".to_string(),
    };

    let handle = kagzi.start_workflow("test-workflow", input).await;
    assert!(handle.is_ok());

    let handle = handle.unwrap();
    assert!(!handle.run_id().to_string().is_empty());
}

#[tokio::test]
#[ignore] // Requires database
async fn test_workflow_handle_status() {
    let kagzi = setup_test_db().await.unwrap();
    kagzi
        .register_workflow("test-workflow", simple_workflow)
        .await;

    let input = TestInput {
        value: "test".to_string(),
    };

    let handle = kagzi.start_workflow("test-workflow", input).await.unwrap();
    let status = handle.status().await;
    assert!(status.is_ok());

    let run = status.unwrap();
    assert_eq!(run.workflow_name, "test-workflow");
}

#[tokio::test]
#[ignore] // Requires database
async fn test_workflow_execution() {
    let kagzi = setup_test_db().await.unwrap();
    kagzi
        .register_workflow("test-workflow", simple_workflow)
        .await;

    let input = TestInput {
        value: "integration-test".to_string(),
    };

    let handle = kagzi.start_workflow("test-workflow", input).await.unwrap();

    // Create and start a worker to execute the workflow
    let worker = kagzi.create_worker();
    let worker_handle = tokio::spawn(async move {
        // Run worker for a short time
        tokio::select! {
            _ = worker.start() => {},
            _ = tokio::time::sleep(std::time::Duration::from_secs(2)) => {},
        }
    });

    // Wait for the result with a timeout
    let result = tokio::time::timeout(std::time::Duration::from_secs(3), handle.result()).await;

    // Clean up worker
    worker_handle.abort();

    assert!(result.is_ok());
    let workflow_result = result.unwrap();
    assert!(workflow_result.is_ok());

    let output = workflow_result.unwrap();
    assert_eq!(output, serde_json::json!("Processed: integration-test"));
}

#[tokio::test]
#[ignore] // Requires database
async fn test_workflow_with_steps() {
    let kagzi = setup_test_db().await.unwrap();
    kagzi
        .register_workflow("step-workflow", workflow_with_step)
        .await;

    let input = TestInput {
        value: "step-test".to_string(),
    };

    let handle = kagzi.start_workflow("step-workflow", input).await.unwrap();

    // Create and start a worker
    let worker = kagzi.create_worker();
    let worker_handle = tokio::spawn(async move {
        tokio::select! {
            _ = worker.start() => {},
            _ = tokio::time::sleep(std::time::Duration::from_secs(2)) => {},
        }
    });

    // Wait for result
    let result = tokio::time::timeout(std::time::Duration::from_secs(3), handle.result()).await;

    worker_handle.abort();

    assert!(result.is_ok());
    let workflow_result = result.unwrap();
    assert!(workflow_result.is_ok());

    let output = workflow_result.unwrap();
    assert_eq!(output, serde_json::json!("Step result: step-test"));
}

#[tokio::test]
#[ignore] // Requires database
async fn test_health_check() {
    let kagzi = setup_test_db().await.unwrap();
    let health = kagzi.health_check().await;
    assert!(health.is_ok());
}

#[tokio::test]
#[ignore] // Requires database
async fn test_cancel_workflow() {
    let kagzi = setup_test_db().await.unwrap();
    kagzi
        .register_workflow("test-workflow", simple_workflow)
        .await;

    let input = TestInput {
        value: "test".to_string(),
    };

    let handle = kagzi.start_workflow("test-workflow", input).await.unwrap();

    // Cancel the workflow
    let cancel_result = handle.cancel().await;
    assert!(cancel_result.is_ok());

    // Check that it was cancelled
    let status = handle.status().await.unwrap();
    assert_eq!(status.status, kagzi_core::WorkflowStatus::Cancelled);
}
