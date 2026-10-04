use phantom_core::{
    Config, Result, debug_warn,
    diagnostics::log::{
        ConsoleFormat, ConsoleWriter, LogLevelReloadHandles, ansi_enabled, capture, fmt_span,
    },
    err,
};
use tracing::{Subscriber, subscriber::NoSubscriber};
use tracing_subscriber::{
    EnvFilter, Layer, Registry, fmt, layer::SubscriberExt, registry::LookupSpan, reload,
};
