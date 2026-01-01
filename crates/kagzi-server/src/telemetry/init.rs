//! Telemetry initialization for the Kagzi server.
//!
//! Sets up OpenTelemetry with:
//! - Trace provider (stdout exporter)
//! - Metrics provider (stdout exporter)
//! - W3C Trace Context propagation
//! - tracing-subscriber integration

use opentelemetry::propagation::TextMapCompositePropagator;
use opentelemetry::trace::TracerProvider as _;
use opentelemetry::{KeyValue, global};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::metrics::SdkMeterProvider;
use opentelemetry_sdk::propagation::TraceContextPropagator;
use opentelemetry_sdk::trace::SdkTracerProvider;
use opentelemetry_semantic_conventions::resource::SERVICE_VERSION;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, fmt};

use crate::config::TelemetrySettings;

/// Guard that ensures proper shutdown of OpenTelemetry providers.
/// Drop this guard to flush and shutdown all telemetry.
pub struct TelemetryGuard {
    tracer_provider: Option<SdkTracerProvider>,
    meter_provider: Option<SdkMeterProvider>,
}

impl Drop for TelemetryGuard {
    fn drop(&mut self) {
        if let Some(ref provider) = self.tracer_provider
            && let Err(e) = provider.shutdown()
        {
            eprintln!("Failed to shutdown tracer provider: {:?}", e);
        }
        if let Some(ref provider) = self.meter_provider
            && let Err(e) = provider.shutdown()
        {
            eprintln!("Failed to shutdown meter provider: {:?}", e);
        }
    }
}

/// Initialize telemetry with OpenTelemetry integration.
///
/// Returns a guard that should be kept alive for the duration of the program.
/// When dropped, it will flush and shutdown all telemetry providers.
pub fn init_telemetry(settings: &TelemetrySettings) -> anyhow::Result<TelemetryGuard> {
    // Build the resource with service info
    let resource = Resource::builder()
        .with_service_name(settings.service_name.clone())
        .with_attribute(KeyValue::new(SERVICE_VERSION, env!("CARGO_PKG_VERSION")))
        .build();

    // Set up the env filter
    let env_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(&settings.log_level));

    if settings.enabled {
        // Initialize W3C Trace Context propagator for distributed tracing
        let propagator =
            TextMapCompositePropagator::new(vec![Box::new(TraceContextPropagator::new())]);
        global::set_text_map_propagator(propagator);

        // Initialize OpenTelemetry providers
        let (tracer_provider, meter_provider) = init_otel_providers(resource.clone());

        // Get a tracer from the provider
        let tracer = tracer_provider.tracer("kagzi-server");

        // Set global providers
        global::set_tracer_provider(tracer_provider.clone());
        global::set_meter_provider(meter_provider.clone());

        // Create the OpenTelemetry layer for tracing
        let otel_layer = tracing_opentelemetry::layer().with_tracer(tracer);

        // Build the subscriber with both fmt and OpenTelemetry layers
        match settings.log_format.as_str() {
            "json" => {
                tracing_subscriber::registry()
                    .with(env_filter)
                    .with(otel_layer)
                    .with(fmt::layer().json())
                    .init();
            }
            _ => {
                tracing_subscriber::registry()
                    .with(env_filter)
                    .with(otel_layer)
                    .with(fmt::layer())
                    .init();
            }
        }

        tracing::info!(
            service.name = %settings.service_name,
            otel.enabled = true,
            "Telemetry initialized with OpenTelemetry exporters (stdout)"
        );

        Ok(TelemetryGuard {
            tracer_provider: Some(tracer_provider),
            meter_provider: Some(meter_provider),
        })
    } else {
        // OTEL disabled - just use basic tracing subscriber
        match settings.log_format.as_str() {
            "json" => {
                tracing_subscriber::registry()
                    .with(env_filter)
                    .with(fmt::layer().json())
                    .init();
            }
            _ => {
                tracing_subscriber::registry()
                    .with(env_filter)
                    .with(fmt::layer())
                    .init();
            }
        }

        tracing::info!(
            otel.enabled = false,
            "Telemetry initialized (tracing only, OpenTelemetry exporters disabled)"
        );

        Ok(TelemetryGuard {
            tracer_provider: None,
            meter_provider: None,
        })
    }
}

fn init_otel_providers(resource: Resource) -> (SdkTracerProvider, SdkMeterProvider) {
    // Trace provider with stdout exporter (simple for development)
    let trace_exporter = opentelemetry_stdout::SpanExporter::default();
    let tracer_provider = SdkTracerProvider::builder()
        .with_resource(resource.clone())
        .with_simple_exporter(trace_exporter)
        .build();

    // Metrics provider with stdout exporter
    let metrics_exporter = opentelemetry_stdout::MetricExporter::default();
    let reader = opentelemetry_sdk::metrics::PeriodicReader::builder(metrics_exporter).build();
    let meter_provider = SdkMeterProvider::builder()
        .with_resource(resource)
        .with_reader(reader)
        .build();

    (tracer_provider, meter_provider)
}
