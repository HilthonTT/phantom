//! Debug-mode helpers: logging that degrades to `DEBUG` level in release
//! builds, and a hardware breakpoint trap for when phantom runs under a
//! debugger.

use std::{any::Any, env, panic, sync::LazyLock};

// Export debug proc_macros
pub use phantom_macros::recursion_depth;
use tracing::Level;

// Export all of the ancillary tools from here as well.
pub use crate::{Result, utils::result::DebugInspect};

/// Log event at given level in debug-mode (when debug-assertions are enabled).
/// In release-mode it becomes DEBUG level, and possibly subject to elision.
#[macro_export]
macro_rules! debug_event {
    ( $level:expr, $($x:tt)+ ) => {
        if $crate::debug::logging() {
            ::tracing::event!( $level, _debug = true, $($x)+ )
        } else {
            ::tracing::debug!( $($x)+ )
        }
    };
}

/// Log message at the ERROR level in debug-mode (when debug-assertions are
/// enabled). In release-mode it becomes DEBUG level, and possibly subject to
/// elision.
#[macro_export]
macro_rules! debug_error {
    ( $($x:tt)+ ) => {
        $crate::debug_event!(::tracing::Level::ERROR, $($x)+ )
    };
}

/// Log message at the WARN level in debug-mode (when debug-assertions are
/// enabled). In release-mode it becomes DEBUG level, and possibly subject to
/// elision.
#[macro_export]
macro_rules! debug_warn {
    ( $($x:tt)+ ) => {
        $crate::debug_event!(::tracing::Level::WARN, $($x)+ )
    };
}

/// Log message at the INFO level in debug-mode (when debug-assertions are
/// enabled). In release-mode it becomes DEBUG level, and possibly subject to
/// elision.
#[macro_export]
macro_rules! debug_info {
    ( $($x:tt)+ ) => {
        $crate::debug_event!(::tracing::Level::INFO, $($x)+ )
    };
}

pub const INFO_SPAN_LEVEL: Level = if cfg!(debug_assertions) {
    Level::INFO
} else {
    Level::DEBUG
};

/// Whether we appear to be running under a debugger, guessed from the `_`
/// variable the shell sets to the command being executed.
pub static DEBUGGER: LazyLock<bool> =
    LazyLock::new(|| env::var("_").unwrap_or_default().ends_with("gdb"));

/// Installs a panic hook that breaks into the debugger before unwinding.
///
/// `crate::ctor` only exists when the `jemalloc` feature pulls the `ctor`
/// dependency in, so outside that configuration this has to be called by hand
/// from `main`.
#[cfg_attr(all(debug_assertions, feature = "jemalloc"), crate::ctor(unsafe))]
pub fn set_panic_trap() {
    if !*DEBUGGER {
        return;
    }

    let next = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        panic_handler(info, &next);
    }));
}

#[cold]
#[inline(never)]
pub fn panic_handler(info: &panic::PanicHookInfo<'_>, next: &dyn Fn(&panic::PanicHookInfo<'_>)) {
    trap();
    next(info);
}

/// Raises a hardware breakpoint, which a debugger catches and any other process
/// ignores. A no-op on architectures we have no instruction for.
#[inline(always)]
#[allow(unsafe_code)]
pub fn trap() {
    #[cfg(target_arch = "x86_64")]
    // SAFETY: embeds instruction for hardware breakpoint
    unsafe {
        std::arch::asm!("int3");
    }

    #[cfg(target_arch = "aarch64")]
    // SAFETY: embeds instruction for hardware breakpoint
    unsafe {
        std::arch::asm!("brk #0xf000");
    }
}

/// The `&str` a panic carried, or `""` if it carried a formatted message.
#[must_use]
pub fn panic_str(p: &(dyn Any + Send)) -> &'static str {
    p.downcast_ref::<&str>().copied().unwrap_or_default()
}

#[inline(always)]
#[must_use]
pub fn rttype_name<T: ?Sized>(_: &T) -> &'static str {
    type_name::<T>()
}

#[inline(always)]
#[must_use]
pub fn type_name<T: ?Sized>() -> &'static str {
    std::any::type_name::<T>()
}

/// Whether `debug_event!` and friends log at their nominal level rather than
/// collapsing to `DEBUG`.
#[must_use]
#[inline]
pub const fn logging() -> bool {
    cfg!(debug_assertions)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::config::Config;

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

    #[test]
    fn type_names_resolve() {
        assert!(type_name::<Config>().ends_with("Config"));
        assert!(rttype_name(&0_u8).ends_with("u8"));
    }

    #[test]
    fn panic_str_reads_str_payloads() {
        let payload = std::panic::catch_unwind(|| panic!("boom")).expect_err("panicked");
        assert_eq!(panic_str(&*payload), "boom");
    }
}
