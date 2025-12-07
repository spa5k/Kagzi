use kagzi_proto::kagzi::HealthCheckRequest;
use kagzi_proto::kagzi::admin_service_client::AdminServiceClient;
use tonic::transport::Channel;
use tracing::info;

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

    let mut client = AdminServiceClient::new(channel);

    let request = tonic::Request::new(HealthCheckRequest {
        service: Some("kagzi-server".to_string()),
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
