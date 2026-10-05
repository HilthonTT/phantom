use std::sync::Arc;

use phantom_core::{
    Config, Result,
    diagnostics::log::{ConsoleFormat, ConsoleWriter, Log, LogLevelReloadHandles, capture},
};
use tracing::Subscriber;
use tracing_subscriber::{
    Layer, Registry, fmt, layer::SubscriberExt, registry::LookupSpan, reload,
};
#[cfg(feature = "perf_measurements")]
use {
    opentelemetry::trace::TracerProvider as _,
    opentelemetry_otlp::{SpanExporter, WithExportConfig as _},
    opentelemetry_sdk::{Resource, propagation::TraceContextPropagator, trace::SdkTracerProvider},
    phantom_core::err,
    tracing_subscriber::EnvFilter,
};

#[cfg(feature = "perf_measurements")]
pub(crate) type TracingFlameGuard =
    Option<tracing_flame::FlushGuard<std::io::BufWriter<std::fs::File>>>;

#[cfg(not(feature = "perf_measurements"))]
pub(crate) type TracingFlameGuard = Option<()>;

/// Installs the global subscriber: the console, the capture layer the admin
/// commands read back, and in a `perf_measurements` build the flame and
/// OpenTelemetry layers.
pub(crate) fn init(config: &Config) -> Result<(TracingFlameGuard, Log)> {
    let reload_handles = LogLevelReloadHandles::default();
    let cap_state = Arc::new(capture::State::new());

    let subscriber = Registry::default()
        .with(console_layer(config, &reload_handles)?)
        .with(capture::Layer::new(&cap_state));

    #[cfg(feature = "perf_measurements")]
    let (subscriber, flame_guard) = {
        let (flame_layer, flame_guard) = tracing_flame_layer(config)?;
        let jaeger_layer = opentelemetry_layer(config, &reload_handles)?;
        let subscriber = subscriber.with(flame_layer).with(jaeger_layer);

        (subscriber, flame_guard)
    };

    #[cfg(not(feature = "perf_measurements"))]
    let flame_guard = None;

    tracing::subscriber::set_global_default(subscriber)
        .expect("the global default tracing subscriber failed to be initialized");

    Ok((
        flame_guard,
        Log {
            reload: reload_handles,
            capture: cap_state,
        },
    ))
}

fn console_layer<S>(
    config: &Config,
    reload_handles: &LogLevelReloadHandles,
) -> Result<impl Layer<S>>
where
    S: Subscriber + for<'a> LookupSpan<'a> + 'static,
{
    let format = ConsoleFormat::new(config);
    let layer = fmt::Layer::new()
        .with_thread_ids(config.logging.log_thread_ids)
        .with_span_events(config.span_events()?)
        .event_format(format.clone())
        .fmt_fields(format)
        .with_writer(ConsoleWriter::new(config));

    let (reload_filter, reload_handle) = reload::Layer::new(config.log_filter()?);

    reload_handles.add("console", reload_handle);
    Ok(layer.with_filter(reload_filter))
}

#[cfg(feature = "perf_measurements")]
fn tracing_flame_layer<S>(config: &Config) -> Result<(Option<impl Layer<S>>, TracingFlameGuard)>
where
    S: Subscriber + for<'a> LookupSpan<'a> + 'static,
{
    let logging = &config.logging;
    if !logging.tracing_flame {
        return Ok((None, None));
    }

    let filter = EnvFilter::try_new(&logging.tracing_flame_filter)
        .map_err(|e| err!(Config("tracing_flame_filter", "{e}.")))?;

    let (layer, guard) = tracing_flame::FlameLayer::with_file(&logging.tracing_flame_output_path)
        .map_err(|e| err!(Config("tracing_flame_output_path", "{e}.")))?;

    let layer = layer.with_empty_samples(false).with_filter(filter);

    Ok((Some(layer), Some(guard)))
}

#[cfg(feature = "perf_measurements")]
fn opentelemetry_layer<S>(
    config: &Config,
    reload_handles: &LogLevelReloadHandles,
) -> Result<Option<impl Layer<S>>>
where
    S: Subscriber + for<'a> LookupSpan<'a> + 'static,
{
    let logging = &config.logging;
    if !logging.allow_jaeger {
        return Ok(None);
    }

    let filter = EnvFilter::try_new(&logging.jaeger_filter)
        .map_err(|e| err!(Config("jaeger_filter", "{e}.")))?;

    opentelemetry::global::set_text_map_propagator(TraceContextPropagator::new());

    // OTLP over HTTP; the endpoint comes from OTEL_EXPORTER_OTLP_ENDPOINT and
    // defaults to the collector's localhost:4318.
    let exporter = SpanExporter::builder()
        .with_http()
        .with_protocol(opentelemetry_otlp::Protocol::HttpBinary)
        .build()
        .map_err(|e| err!(Config("allow_jaeger", "OTLP span exporter: {e}.")))?;

    let resource = Resource::builder().with_service_name("phantom").build();

    let provider = SdkTracerProvider::builder()
        .with_batch_exporter(exporter)
        .with_resource(resource)
        .build();

    let tracer = provider.tracer("phantom");
    let telemetry = tracing_opentelemetry::layer().with_tracer(tracer);

    let (reload_filter, reload_handle) = reload::Layer::new(filter);

    reload_handles.add("jaeger", reload_handle);
    Ok(Some(telemetry.with_filter(reload_filter)))
}
