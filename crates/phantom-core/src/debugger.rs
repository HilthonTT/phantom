use std::{env, panic, sync::LazyLock};

pub static DEBUGGER: LazyLock<bool> =
    LazyLock::new(|| env::var("_").unwrap_or_default().ends_with("gdb"));

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
