//! Logging that is loud in debug builds and quiet in release ones.
//!
//! The macros here log at their nominal level when debug assertions are on and
//! collapse to `DEBUG` when they are not, so a message that is worth an
//! `ERROR` while developing does not become one in production. Everything
//! keyed off that decision lives here; the debugger integration itself is in
//! [`crate::debugger`].

use tracing::Level;

/// Log event at given level in debug-mode (when debug-assertions are enabled).
/// In release-mode it becomes DEBUG level, and possibly subject to elision.
#[macro_export]
macro_rules! debug_event {
    ( $level:expr, $($x:tt)+ ) => {
        if $crate::log::debug::logging() {
            $crate::tracing::event!( $level, _debug = true, $($x)+ )
        } else {
            $crate::tracing::debug!( $($x)+ )
        }
    };
}

/// Log message at the ERROR level in debug-mode (when debug-assertions are
/// enabled). In release-mode it becomes DEBUG level, and possibly subject to
/// elision.
#[macro_export]
macro_rules! debug_error {
    ( $($x:tt)+ ) => {
        $crate::debug_event!($crate::tracing::Level::ERROR, $($x)+ )
    };
}

/// Log message at the WARN level in debug-mode (when debug-assertions are
/// enabled). In release-mode it becomes DEBUG level, and possibly subject to
/// elision.
#[macro_export]
macro_rules! debug_warn {
    ( $($x:tt)+ ) => {
        $crate::debug_event!($crate::tracing::Level::WARN, $($x)+ )
    };
}

/// Log message at the INFO level in debug-mode (when debug-assertions are
/// enabled). In release-mode it becomes DEBUG level, and possibly subject to
/// elision.
#[macro_export]
macro_rules! debug_info {
    ( $($x:tt)+ ) => {
        $crate::debug_event!($crate::tracing::Level::INFO, $($x)+ )
    };
}

/// The level an `#[instrument]` span carries when it should be visible while
/// developing but not in production.
pub const INFO_SPAN_LEVEL: Level = if cfg!(debug_assertions) {
    Level::INFO
} else {
    Level::DEBUG
};

/// Whether [`debug_event!`] and friends log at their nominal level rather than
/// collapsing to `DEBUG`.
#[must_use]
#[inline]
pub const fn logging() -> bool {
    cfg!(debug_assertions)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_macros_expand() {
        let value = 42;
        debug_error!("error {value}");
        debug_warn!(?value, "warn");
        debug_info!("info {}", value);
        debug_event!(Level::TRACE, "trace {value}");
    }

    #[test]
    fn logging_tracks_debug_assertions() {
        assert_eq!(logging(), cfg!(debug_assertions));
        assert_eq!(
            INFO_SPAN_LEVEL,
            if logging() { Level::INFO } else { Level::DEBUG }
        );
    }
}
