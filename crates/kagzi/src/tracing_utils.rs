use std::cell::RefCell;
use tonic::{Request, metadata::MetadataKey};
use uuid::Uuid;

pub const CORRELATION_ID_KEY: &str = "x-correlation-id";

pub const TRACE_ID_KEY: &str = "x-trace-id";

// Task-local storage for tracing context
tokio::task_local! {
    static CORRELATION_ID: RefCell<Option<String>>;
    static TRACE_ID: RefCell<Option<String>>;
}

pub fn set_correlation_id(id: String) {
    let _ = CORRELATION_ID.try_with(|cell| {
        *cell.borrow_mut() = Some(id);
    });
}

pub fn set_trace_id(id: String) {
    let _ = TRACE_ID.try_with(|cell| {
        *cell.borrow_mut() = Some(id);
    });
}

pub fn get_or_generate_correlation_id() -> String {
    CORRELATION_ID
        .try_with(|cell| cell.borrow().clone())
        .ok()
        .flatten()
        .unwrap_or_else(|| Uuid::new_v4().to_string())
}

pub fn get_or_generate_trace_id() -> String {
    TRACE_ID
        .try_with(|cell| cell.borrow().clone())
        .ok()
        .flatten()
        .unwrap_or_else(|| Uuid::new_v4().to_string())
}

pub async fn with_tracing_context<F, T>(
    correlation_id: Option<String>,
    trace_id: Option<String>,
    f: F,
) -> T
where
    F: std::future::Future<Output = T>,
{
    let correlation_id = correlation_id.unwrap_or_else(|| Uuid::new_v4().to_string());
    let trace_id = trace_id.unwrap_or_else(|| Uuid::new_v4().to_string());

    CORRELATION_ID
        .scope(RefCell::new(Some(correlation_id)), async {
            TRACE_ID.scope(RefCell::new(Some(trace_id)), f).await
        })
        .await
}

pub fn extract_tracing_context<T>(request: &Request<T>) -> (String, String) {
    let metadata = request.metadata();

    let correlation_id = metadata
        .get(CORRELATION_ID_KEY)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let trace_id = metadata
        .get(TRACE_ID_KEY)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    (correlation_id, trace_id)
}

pub fn add_tracing_metadata<T>(request: Request<T>) -> Request<T> {
    let correlation_id = get_or_generate_correlation_id();
    let trace_id = get_or_generate_trace_id();

    let mut request = request;
    let metadata = request.metadata_mut();

    if let Ok(correlation_key) = MetadataKey::from_bytes(CORRELATION_ID_KEY.as_bytes())
        && let Ok(value) = correlation_id.parse()
    {
        metadata.insert(correlation_key, value);
    }

    if let Ok(trace_key) = MetadataKey::from_bytes(TRACE_ID_KEY.as_bytes())
        && let Ok(value) = trace_id.parse()
    {
        metadata.insert(trace_key, value);
    }

    request
}
#[allow(dead_code)]
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
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_tracing_context_propagation() {
        let correlation_id = "test-correlation-123".to_string();
        let trace_id = "test-trace-456".to_string();

        let (captured_correlation, captured_trace) = with_tracing_context(
            Some(correlation_id.clone()),
            Some(trace_id.clone()),
            async { (get_or_generate_correlation_id(), get_or_generate_trace_id()) },
        )
        .await;

        assert_eq!(captured_correlation, correlation_id);
        assert_eq!(captured_trace, trace_id);
    }

    #[tokio::test]
    async fn test_generates_new_ids_when_not_set() {
        // Outside of tracing context, should generate new UUIDs
        let id1 = get_or_generate_correlation_id();
        let id2 = get_or_generate_correlation_id();

        // Both should be valid UUIDs (though potentially different)
        assert!(uuid::Uuid::parse_str(&id1).is_ok());
        assert!(uuid::Uuid::parse_str(&id2).is_ok());
    }
}
