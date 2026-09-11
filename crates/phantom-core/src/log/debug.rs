use tracing::Level;

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

#[macro_export]
macro_rules! debug_error {
    ( $($x:tt)+ ) => {
        $crate::debug_event!($crate::tracing::Level::ERROR, $($x)+ )
    };
}

#[macro_export]
macro_rules! debug_warn {
    ( $($x:tt)+ ) => {
        $crate::debug_event!($crate::tracing::Level::WARN, $($x)+ )
    };
}

#[macro_export]
macro_rules! debug_info {
    ( $($x:tt)+ ) => {
        $crate::debug_event!($crate::tracing::Level::INFO, $($x)+ )
    };
}

pub const INFO_SPAN_LEVEL: Level = if cfg!(debug_assertions) {
    Level::INFO
} else {
    Level::DEBUG
};

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
