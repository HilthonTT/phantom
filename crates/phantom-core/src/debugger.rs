//! Running phantom under a debugger.
//!
//! A hardware breakpoint instruction is a no-op to any process that is not
//! being traced, so the panic hook installed here can trap unconditionally:
//! under `gdb` it stops at the panic site with the stack still intact, and
//! anywhere else it is ignored. The debug-level logging that pairs with this
//! is in [`crate::log::debug`].

use std::{env, panic, sync::LazyLock};

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
    unsafe {
        std::arch::asm!("int3");
    }

    #[cfg(target_arch = "aarch64")]
    unsafe {
        std::arch::asm!("brk #0xf000");
    }
}
