//! W3C Trace Context propagation for gRPC.
//!
//! Provides utilities to inject trace context into outgoing gRPC requests,
//! enabling distributed tracing between the SDK and server.

use opentelemetry::propagation::Injector;
use opentelemetry::{Context, global};
use tonic::metadata::{MetadataKey, MetadataMap, MetadataValue};

/// Injects the current trace context into outgoing gRPC request metadata.
///
/// Uses W3C Trace Context format (`traceparent`, `tracestate` headers).
/// Call this before making outgoing gRPC calls to propagate the trace.
pub fn inject_context(metadata: &mut MetadataMap) {
    let cx = Context::current();
    global::get_text_map_propagator(|propagator| {
        propagator.inject_context(&cx, &mut MetadataInjector(metadata))
    });
}

/// Injector for writing headers to gRPC MetadataMap.
struct MetadataInjector<'a>(&'a mut MetadataMap);

impl Injector for MetadataInjector<'_> {
    fn set(&mut self, key: &str, value: String) {
        if let Ok(name) = MetadataKey::from_bytes(key.as_bytes())
            && let Ok(val) = MetadataValue::try_from(&value)
        {
            self.0.insert(name, val);
        }
    }
}
