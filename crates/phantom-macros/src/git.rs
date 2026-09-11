use std::{process::Command, str};

use proc_macro::TokenStream;
use quote::quote;

pub(super) fn semantic(_args: TokenStream) -> TokenStream {
    let output = git(&["describe", "--tags", "--abbrev=1"]);

    let output = output.strip_prefix('v').unwrap_or(&output);
    let output = output.rsplit_once('-').map_or(output, |(head, _)| head);

    quote! {
        static GIT_SEMANTIC: &'static str = #output;
    }
    .into()
}

pub(super) fn commit(_args: TokenStream) -> TokenStream {
    let output = git(&["describe", "--always", "--dirty", "--abbrev=10"]);

    quote! {
        static GIT_COMMIT: &'static str = #output;
    }
    .into()
}

fn git(args: &[&str]) -> String {
    Command::new("git")
        .args(args)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| {
            str::from_utf8(&output.stdout)
                .map(str::trim)
                .map(String::from)
                .ok()
        })
        .unwrap_or_default()
}
