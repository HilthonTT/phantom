//! Logging macros.
//!
//! Prefer these over reaching for `::tracing` or `::log` directly in project
//! code, so that the crate keeps one place to change how logging is dispatched.
//! The `debug_*` variants in [`crate::debug`] are exported to the crate
//! namespace alongside these.

pub use tracing::Level;

#[macro_export]
macro_rules! event {
    ( $level:expr, $($x:tt)+ ) => { ::tracing::event!( $level, $($x)+ ) };
}

#[macro_export]
macro_rules! error {
    ( $($x:tt)+ ) => { ::tracing::error!( $($x)+ ) };
}

#[macro_export]
macro_rules! warn {
    ( $($x:tt)+ ) => { ::tracing::warn!( $($x)+ ) };
}

#[macro_export]
macro_rules! info {
    ( $($x:tt)+ ) => { ::tracing::info!( $($x)+ ) };
}

#[macro_export]
macro_rules! debug {
    ( $($x:tt)+ ) => { ::tracing::debug!( $($x)+ ) };
}

#[macro_export]
macro_rules! trace {
    ( $($x:tt)+ ) => { ::tracing::trace!( $($x)+ ) };
}
