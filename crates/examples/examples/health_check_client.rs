use kagzi_proto::kagzi::HealthCheckRequest;
use kagzi_proto::kagzi::workflow_service_client::WorkflowServiceClient;
use tonic::transport::Channel;
use tracing::info;

/// Application entry point that initializes structured tracing, connects to the workflow gRPC server,
/// performs a health check for "kagzi-server", and logs the returned status and message.
///
/// # Examples
///
/// ```no_run
/// # use std::error::Error;
/// /// Call the program's async entry point (runs the same initialization and health check).
/// #[tokio::main]
/// async fn example() -> Result<(), Box<dyn Error>> {
///     crate::main().await?;
///     Ok(())
/// }
/// ```
///
/// # Returns
///
/// `Ok(())` on success, `Err` if establishing the gRPC connection or performing the RPC fails.
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .json()
        .with_current_span(false)
        .with_span_list(true)
        .with_target(true)
        .with_thread_ids(true)
        .with_thread_names(true)
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let channel = Channel::from_static("http://localhost:50051")
        .connect()
        .await?;

    let mut client = WorkflowServiceClient::new(channel);

    let request = tonic::Request::new(HealthCheckRequest {
        service: "kagzi-server".to_string(),
    });

    match client.health_check(request).await {
        Ok(response) => {
            let health = response.into_inner();
            info!(
                status = ?health.status,
                message = %health.message,
                "Health check response received"
            );
        }
        Err(e) => {
            tracing::error!(error = %e, "Health check failed");
        }
    }

    Ok(())
}