use tonic::{Request, metadata::MetadataKey};
use tracing::Span;
use uuid::Uuid;

/// Correlation ID metadata key for gRPC
pub const CORRELATION_ID_KEY: &str = "x-correlation-id";

/// Trace ID metadata key for distributed tracing  
pub const TRACE_ID_KEY: &str = "x-trace-id";

/// Generate or extract correlation ID for current context
pub fn get_or_generate_correlation_id() -> String {
    if let Some(correlation_id) = Span::current().field("correlation_id")
        && let Some(correlation_id_str) = correlation_id.to_string().strip_prefix("\"")
        && let Some(clean_id) = correlation_id_str.strip_suffix("\"")
    {
        return clean_id.to_string();
    }

    Uuid::new_v4().to_string()
}

/// Generate or extract trace ID for current context
pub fn get_or_generate_trace_id() -> String {
    if let Some(trace_id) = Span::current().field("trace_id")
        && let Some(trace_id_str) = trace_id.to_string().strip_prefix("\"")
        && let Some(clean_id) = trace_id_str.strip_suffix("\"")
    {
        return clean_id.to_string();
    }

    Uuid::new_v4().to_string()
}

/// Add tracing metadata to gRPC request
pub fn add_tracing_metadata<T>(request: Request<T>) -> Request<T> {
    let correlation_id = get_or_generate_correlation_id();
    let trace_id = get_or_generate_trace_id();

    let mut request = request;
    let metadata = request.metadata_mut();

    if let Ok(correlation_key) = MetadataKey::from_bytes(CORRELATION_ID_KEY.as_bytes()) {
        metadata.insert(correlation_key, correlation_id.parse().unwrap());
    }

    if let Ok(trace_key) = MetadataKey::from_bytes(TRACE_ID_KEY.as_bytes()) {
        metadata.insert(trace_key, trace_id.parse().unwrap());
    }

    request
}

#[allow(dead_code)]
/// Initialize tracing for SDK
pub fn init_tracing() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .json()
        .with_current_span(false)
        .with_span_list(true)
        .with_target(true)
        .with_thread_ids(true)
        .with_thread_names(true)
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    Ok(())
}
