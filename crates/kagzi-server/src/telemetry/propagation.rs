//! W3C Trace Context propagation for gRPC.
//!
//! Provides utilities to extract and inject trace context from/to gRPC metadata,
//! enabling distributed tracing across services and SDKs in any language.

use opentelemetry::propagation::{Extractor, Injector};
use opentelemetry::{Context, global};
use tonic::metadata::{MetadataKey, MetadataMap, MetadataValue};

/// Extracts trace context from incoming gRPC request metadata.
///
/// Uses W3C Trace Context format (`traceparent`, `tracestate` headers).
/// Returns a new Context with the extracted span context, or the current context if none found.
///
/// # Example
/// ```no_run
/// async fn my_grpc_method(&self, request: Request<MyRequest>) -> Result<Response<MyResponse>, Status> {
///     let parent_cx = extract_context(request.metadata());
///     let _guard = parent_cx.attach();
///     // ... handle request, spans created here will be children of the extracted context
/// }
/// ```
pub fn extract_context(metadata: &MetadataMap) -> Context {
    global::get_text_map_propagator(|propagator| propagator.extract(&MetadataExtractor(metadata)))
}

/// Injects the current trace context into outgoing gRPC request metadata.
///
/// Uses W3C Trace Context format (`traceparent`, `tracestate` headers).
/// Call this before making outgoing gRPC calls to propagate the trace.
///
/// # Example
/// ```no_run
/// let mut request = tonic::Request::new(my_request);
/// inject_context(request.metadata_mut());
/// let response = client.some_method(request).await?;
/// ```
pub fn inject_context(metadata: &mut MetadataMap) {
    let cx = Context::current();
    global::get_text_map_propagator(|propagator| {
        propagator.inject_context(&cx, &mut MetadataInjector(metadata))
    });
}

/// Extractor for reading headers from gRPC MetadataMap.
struct MetadataExtractor<'a>(&'a MetadataMap);

impl Extractor for MetadataExtractor<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(|v| v.to_str().ok())
    }

    fn keys(&self) -> Vec<&str> {
        self.0
            .keys()
            .filter_map(|key| match key {
                tonic::metadata::KeyRef::Ascii(k) => Some(k.as_str()),
                tonic::metadata::KeyRef::Binary(_) => None,
            })
            .collect()
    }
}

/// Injector for writing headers to gRPC MetadataMap.
struct MetadataInjector<'a>(&'a mut MetadataMap);

impl Injector for MetadataInjector<'_> {
    fn set(&mut self, key: &str, value: String) {
        let name = match MetadataKey::from_bytes(key.as_bytes()) {
            Ok(name) => name,
            Err(e) => {
                tracing::debug!("Failed to create metadata key for trace injection: {:?}", e);
                return;
            }
        };
        let val = match MetadataValue::try_from(&value) {
            Ok(val) => val,
            Err(e) => {
                tracing::debug!(
                    "Failed to create metadata value for trace injection: {:?}",
                    e
                );
                return;
            }
        };
        self.0.insert(name, val);
    }
}
