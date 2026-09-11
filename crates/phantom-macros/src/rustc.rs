use std::{process::Command, str};

use proc_macro::TokenStream;
use quote::quote;

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

pub(crate) fn is_cargo_test() -> bool {
    std::env::args().any(|flag| flag == "--test")
}

pub(crate) fn flags_capture(args: TokenStream) -> TokenStream {
    let Some(crate_name) = crate::attribute::get_crate_name() else {
        return args;
    };

    let flags = std::env::args().collect::<Vec<_>>();
    let len = flags.len();

    quote! {

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
