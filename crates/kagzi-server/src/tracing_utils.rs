use tonic::Request;
use tracing_subscriber::prelude::*;
use uuid::Uuid;

pub const CORRELATION_ID_KEY: &str = "x-correlation-id";

pub const TRACE_ID_KEY: &str = "x-trace-id";

pub fn extract_or_generate_correlation_id<T>(request: &Request<T>) -> String {
    let metadata = request.metadata();

    if let Some(correlation_id) = metadata.get(CORRELATION_ID_KEY)
        && let Ok(correlation_id_str) = correlation_id.to_str()
    {
        return correlation_id_str.to_string();
    }

    Uuid::now_v7().to_string()
}

pub fn extract_or_generate_trace_id<T>(request: &Request<T>) -> String {
    let metadata = request.metadata();

    if let Some(trace_id) = metadata.get(TRACE_ID_KEY)
        && let Ok(trace_id_str) = trace_id.to_str()
    {
        return trace_id_str.to_string();
    }

    Uuid::now_v7().to_string()
}
pub fn log_grpc_request(
    method: &str,
    correlation_id: &str,
    trace_id: &str,
    request_body: Option<&str>,
) {
    tracing::info!(
        method = method,
        correlation_id = correlation_id,
        trace_id = trace_id,
        "gRPC request received"
    );

    if let Some(body) = request_body {
        tracing::trace!(
            method = method,
            correlation_id = correlation_id,
            trace_id = trace_id,
            request_body = body,
            "gRPC request details"
        );
    }
}

pub fn log_grpc_response(
    method: &str,
    correlation_id: &str,
    trace_id: &str,
    status_code: tonic::Code,
    message: Option<&str>,
) {
    match status_code {
        tonic::Code::Ok => {
            tracing::info!(
                method = method,
                correlation_id = correlation_id,
                trace_id = trace_id,
                "gRPC request completed successfully"
            );
        }
        tonic::Code::NotFound => {
            tracing::warn!(
                method = method,
                correlation_id = correlation_id,
                trace_id = trace_id,
                "gRPC request failed - resource not found"
            );
        }
        tonic::Code::InvalidArgument => {
            tracing::warn!(
                method = method,
                correlation_id = correlation_id,
                trace_id = trace_id,
                error_message = message.unwrap_or("No message"),
                "gRPC request failed - invalid argument"
            );
        }
        tonic::Code::FailedPrecondition => {
            tracing::warn!(
                method = method,
                correlation_id = correlation_id,
                trace_id = trace_id,
                error_message = message.unwrap_or("No message"),
                "gRPC request failed - failed precondition"
            );
        }
        tonic::Code::DeadlineExceeded => {
            tracing::warn!(
                method = method,
                correlation_id = correlation_id,
                trace_id = trace_id,
                "gRPC request failed - deadline exceeded"
            );
        }
        _ => {
            tracing::error!(
                method = method,
                correlation_id = correlation_id,
                trace_id = trace_id,
                status_code = ?status_code,
                error_message = message.unwrap_or("No message"),
                "gRPC request failed - internal error"
            );
        }
    }
}

pub fn init_tracing(service_name: &str) -> Result<(), Box<dyn std::error::Error>> {
    let fmt_layer = tracing_subscriber::fmt::layer()
        .json()
        .with_current_span(false)
        .with_span_list(true)
        .with_target(true)
        .with_thread_ids(true)
        .with_thread_names(true);

    tracing_subscriber::registry()
        .with(fmt_layer)
        .with(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    tracing::info!("Tracing initialized for service: {}", service_name);
    Ok(())
}
