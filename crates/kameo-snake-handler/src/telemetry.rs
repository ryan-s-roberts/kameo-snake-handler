// OpenTelemetry OTLP setup: protocol and endpoint are now fully driven by OTEL_* env vars.
use opentelemetry::global;
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_sdk::{
    metrics::SdkMeterProvider,
    trace::SdkTracerProvider, 
    Resource,
    propagation::TraceContextPropagator,
};
use tracing_subscriber::{layer::SubscriberExt, Registry};
use tracing_opentelemetry::OpenTelemetryLayer;
use opentelemetry_stdout::SpanExporter;
use tracing_subscriber::filter::EnvFilter;
use std::time::Duration;
use metrics_exporter_opentelemetry;
use metrics;

/// Guard to keep the tracer provider alive for the process lifetime.
pub struct OtelGuard {
    tracer_provider: SdkTracerProvider,
    meter_provider: Option<SdkMeterProvider>,
}

impl Drop for OtelGuard {
    fn drop(&mut self) {
        // Ensure all spans are exported on shutdown
        if let Err(e) = self.tracer_provider.shutdown() {
            eprintln!("Failed to shut down tracer provider: {:?}", e);
        }
        
        // Ensure all metrics are exported on shutdown
        if let Some(meter_provider) = self.meter_provider.take() {
            if let Err(e) = meter_provider.shutdown() {
                eprintln!("Failed to shut down meter provider: {:?}", e);
            }
        }
    }
}

/// Configuration for telemetry exporters.
#[derive(Debug, Clone, Copy)]
pub struct TelemetryExportConfig {
    /// Enable OTLP exporter (default: true)
    pub otlp_enabled: bool,
    /// Enable stdout exporter (default: true)
    pub stdout_enabled: bool,
    /// Enable metrics exporter (default: true)
    pub metrics_enabled: bool,
}

impl Default for TelemetryExportConfig {
    fn default() -> Self {
        Self {
            otlp_enabled: true,
            stdout_enabled: true,
            metrics_enabled: true,
        }
    }
}

/// Async OTLP + stdout tracing subscriber setup (for child process, after runtime is running).
/// Both exporters are enabled by default, but can be toggled via config.
pub async fn build_subscriber_with_otel_and_fmt_async_with_config(
    config: TelemetryExportConfig,
) -> (impl tracing::Subscriber + Send + Sync, OtelGuard) {
    tracing::warn!("build_subscriber_with_otel_and_fmt_async CALLED!");
    
    let protocol = std::env::var("OTEL_EXPORTER_OTLP_PROTOCOL").unwrap_or_else(|_| "grpc".to_string());
    
    let mut provider_builder = SdkTracerProvider::builder();

    if config.otlp_enabled {
        tracing::info!(protocol = %protocol, "Adding OTLP exporter");
        // Use the SpanExporter builder which respects OTEL_* env vars
        let builder = opentelemetry_otlp::SpanExporter::builder();
        let exporter = match protocol.as_str() {
            "grpc" => builder.with_tonic().build().expect("Failed to create OTLP span exporter"),
            "http/protobuf" => builder.with_http().build().expect("Failed to create OTLP span exporter"),
            other => panic!("Unknown OTEL_EXPORTER_OTLP_PROTOCOL: {} (expected 'grpc' or 'http/protobuf')", other),
        };
        provider_builder = provider_builder.with_batch_exporter(exporter);
    }
    if config.stdout_enabled {
        tracing::info!("Adding opentelemetry_stdout::SpanExporter for local debug");
        let exporter = SpanExporter::default();
        provider_builder = provider_builder.with_simple_exporter(exporter);
    }

    let tracer_provider = provider_builder
        .with_resource(Resource::builder().build())
        .build();

    tracing::warn!("setup_otel_layer_async CALLED!");
    global::set_tracer_provider(tracer_provider.clone());
    
    // Set the global propagator for distributed tracing
    let propagator = TraceContextPropagator::new();
    global::set_text_map_propagator(propagator);
    
    let tracer = tracer_provider.tracer("rust_trace_exporter");
    let otel_layer = OpenTelemetryLayer::new(tracer);
    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_target(true)
        .with_level(true)
        .with_thread_ids(true)
        .with_thread_names(true)
        .with_ansi(true);

    let subscriber = Registry::default()
        .with(otel_layer)
        .with(fmt_layer)
        .with(EnvFilter::from_default_env());

    // Initialize metrics if enabled
    let meter_provider = if config.metrics_enabled {
        // Setup metrics and get the provider for shutdown
        Some(setup_metrics())
    } else {
        None
    };

    (subscriber, OtelGuard { tracer_provider, meter_provider })
}

/// Sync OTLP + stdout tracing subscriber setup (for parent process, where runtime is already running).
/// Both exporters are enabled by default, but can be toggled via config.
pub fn setup_otel_layer_with_config(config: TelemetryExportConfig) -> impl tracing_subscriber::Layer<tracing_subscriber::Registry> {
    let protocol = std::env::var("OTEL_EXPORTER_OTLP_PROTOCOL").unwrap_or_else(|_| "grpc".to_string());
    
    let mut provider_builder = SdkTracerProvider::builder();

    if config.otlp_enabled {
        tracing::info!(protocol = %protocol, "Adding OTLP exporter");
        // Use the SpanExporter builder which respects OTEL_* env vars
        let builder = opentelemetry_otlp::SpanExporter::builder();
        let exporter = match protocol.as_str() {
            "grpc" => builder.with_tonic().build().expect("Failed to create OTLP span exporter"),
            "http/protobuf" => builder.with_http().build().expect("Failed to create OTLP span exporter"),
            other => panic!("Unknown OTEL_EXPORTER_OTLP_PROTOCOL: {} (expected 'grpc' or 'http/protobuf')", other),
        };
        provider_builder = provider_builder.with_batch_exporter(exporter);
    }
    if config.stdout_enabled {
        tracing::info!("Adding opentelemetry_stdout::SpanExporter for local debug");
        let exporter = SpanExporter::default();
        provider_builder = provider_builder.with_simple_exporter(exporter);
    }

    let tracer_provider = provider_builder
        .with_resource(Resource::builder().build())
        .build();

    let tracer = tracer_provider.tracer("rust_trace_exporter");
    global::set_tracer_provider(tracer_provider);
    
    // Set the global propagator for distributed tracing
    let propagator = TraceContextPropagator::new();
    global::set_text_map_propagator(propagator);
    
    // Initialize metrics if enabled
    if config.metrics_enabled {
        // Setup metrics and discard the provider since we don't need to handle shutdown
        // in the sync case (the process will handle it)
        let _ = setup_metrics();
    }
    
    tracing_opentelemetry::layer().with_tracer(tracer)
}

/// Initialize OpenTelemetry metrics with OTLP exporter.
/// 
/// This function sets up the OpenTelemetry metrics pipeline:
/// 1. Creates an OTLP metrics exporter using environment variables for configuration
/// 2. Sets up a periodic reader to collect and export metrics
/// 3. Configures the global meter provider
/// 4. Bridges the metrics crate with OpenTelemetry using metrics-exporter-opentelemetry
///
/// Environment variables that affect the configuration:
/// - OTEL_EXPORTER_OTLP_ENDPOINT: The endpoint to send metrics to (default: http://localhost:4317)
/// - OTEL_EXPORTER_OTLP_PROTOCOL: The protocol to use (grpc or http/protobuf, default: grpc)
/// 
/// Returns the SdkMeterProvider for proper shutdown handling.
fn setup_metrics() -> SdkMeterProvider {
    tracing::info!("Initializing OpenTelemetry metrics with OTLP exporter");
    
    // Create a metrics exporter - this will automatically use OTEL_* environment variables
    let exporter = opentelemetry_otlp::MetricExporter::builder()
        .with_tonic() // Default to gRPC, environment variables will override if needed
        .build()
        .map_err(|e| {
            tracing::error!(error = %e, "Failed to create OTLP metrics exporter");
            e
        })
        .expect("Failed to create OTLP metrics exporter");
    
    // Create a periodic reader with the exporter
    let reader = opentelemetry_sdk::metrics::PeriodicReader::builder(exporter)
        .with_interval(Duration::from_secs(5))
        .build();
    
    // Build the meter provider with the reader
    let provider = opentelemetry_sdk::metrics::SdkMeterProvider::builder()
        .with_reader(reader)
        .with_resource(Resource::builder().build())
        .build();
        
    // Set the global meter provider
    global::set_meter_provider(provider.clone());
    
    // Get a meter from the global provider
    let meter = global::meter("kameo-snake-handler");
    
    // Set up the metrics-exporter-opentelemetry bridge
    // This will connect the metrics crate to the OpenTelemetry SDK
    let recorder = metrics_exporter_opentelemetry::Recorder::with_meter(meter);
    
    // Install the recorder
    if let Err(err) = metrics::set_global_recorder(recorder) {
        tracing::error!(error = %err, "Failed to install metrics-exporter-opentelemetry recorder");
    } else {
        tracing::info!("OTLP metrics exporter and metrics-opentelemetry bridge configured");
    }
    
    provider
}
