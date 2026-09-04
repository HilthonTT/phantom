//! What rustc was asked to do, read back from its own command line.
//!
//! `#[config_example_generator]` writes a file as a side effect, and doing
//! that on every metadata-only pass would have an editor rewrite it on each
//! keystroke. [`is_cargo_build`] and [`is_cargo_test`] say whether this
//! invocation is one that should.
//!
//! The same command line is also where a build's cargo features and its
//! compiler version can be read, which is what [`flags_capture`] and
//! [`version`] bake into the binary for the admin surface to report.

use std::{process::Command, str};

use proc_macro::TokenStream;
use quote::quote;

/// True when rustc was invoked to actually link a binary, as opposed to the
/// metadata-only passes `cargo check`, rust-analyzer and friends run.
///
/// The example file is only written on real builds so that editors re-checking
/// the crate on every keystroke do not keep rewriting it.
pub(crate) fn is_cargo_build() -> bool {
    let mut args = std::env::args();

    while let Some(arg) = args.next() {
        let kinds = if let Some(kinds) = arg.strip_prefix("--emit=") {
            kinds.to_owned()
        } else if arg == "--emit" {
            let Some(kinds) = args.next() else {
                break;
            };

            kinds
        } else {
            continue;
        };

        if kinds
            .split(',')
            .any(|kind| kind.split_once('=').map_or(kind, |(kind, _)| kind) == "link")
        {
            return true;
        }
    }

    false
}

/// True when rustc is building a test harness rather than the library itself.
pub(crate) fn is_cargo_test() -> bool {
    std::env::args().any(|flag| flag == "--test")
}

/// Defines `RUSTC_FLAGS` and registers it with [`phantom_core::info::rustc`].
///
/// The flags are rustc's own command line, which is where the cargo features
/// this crate was actually built with end up — a build's enabled features are
/// otherwise not visible to it at runtime, since `cfg!` can only answer for
/// features named in the source.
///
/// Registration is a pre-main constructor rather than a call the binary makes,
/// so a crate opts in by invoking this and nothing has to be kept in step with
/// the list of crates.
pub(crate) fn flags_capture(args: TokenStream) -> TokenStream {
    let Some(crate_name) = crate::attribute::get_crate_name() else {
        return args;
    };

    let flags = std::env::args().collect::<Vec<_>>();
    let len = flags.len();

    quote! {
        /// The compiler arguments this crate was built with.
        pub static RUSTC_FLAGS: [&str; #len] = [#(#flags),*];

        #[phantom_core::ctor(unsafe)]
        fn _set_rustc_flags() {
            phantom_core::info::rustc::FLAGS
                .lock()
                .expect("the rustc flag registry is never held across a panic")
                .insert(#crate_name, &RUSTC_FLAGS);
        }
    }
    .into()
}

/// Defines `RUSTC_VERSION`: the output of `rustc -V` for the compiler running
/// this build, or the empty string where it could not be asked.
///
/// The compiler is found from rustc's own argv rather than from `PATH`, so a
/// toolchain selected by `rust-toolchain.toml` reports itself rather than
/// whatever `rustc` resolves to at build time.
pub(crate) fn version(args: TokenStream) -> TokenStream {
    if crate::attribute::get_crate_name().is_none() {
        return args;
    }

    let version = std::env::args()
        .next()
        .and_then(|rustc| Command::new(rustc).arg("-V").output().ok())
        .filter(|output| output.status.success())
        .and_then(|output| {
            str::from_utf8(&output.stdout)
                .map(str::trim)
                .map(String::from)
                .ok()
        })
        .unwrap_or_default();

    quote! {
        static RUSTC_VERSION: &'static str = #version;
    }
    .into()
}
