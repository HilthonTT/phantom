//! Reading the version out of git at compile time.
//!
//! A release build is identified by the tag it was cut from and a development
//! build by the commit it came from, and neither is in the source tree — they
//! are in the repository around it. `git` is run during the build and the
//! answers baked in, so a binary can say what it was built from without the
//! checkout being there any more.
//!
//! A build from a tarball, or one where `git` is not installed, yields the
//! empty string rather than failing: the version is a courtesy, and refusing to
//! compile without a repository would make the source unbuildable on its own.

use std::{process::Command, str};

use proc_macro::TokenStream;
use quote::quote;

/// Defines `GIT_SEMANTIC`: the nearest tag, stripped to a bare version.
///
/// `v0.1.0-3-gabc` becomes `0.1.0`, so a build between releases reports the
/// release it is descended from rather than a string no version parser accepts.
pub(super) fn semantic(_args: TokenStream) -> TokenStream {
    let output = git(&["describe", "--tags", "--abbrev=1"]);

    // `v0.1.0-3-gabc` -> `0.1.0-3` -> `0.1.0`
    let output = output.strip_prefix('v').unwrap_or(&output);
    let output = output.rsplit_once('-').map_or(output, |(head, _)| head);

    quote! {
        static GIT_SEMANTIC: &'static str = #output;
    }
    .into()
}

/// Defines `GIT_COMMIT`: the commit this was built from, `-dirty` where the
/// tree had uncommitted changes.
pub(super) fn commit(_args: TokenStream) -> TokenStream {
    let output = git(&["describe", "--always", "--dirty", "--abbrev=10"]);

    quote! {
        static GIT_COMMIT: &'static str = #output;
    }
    .into()
}

/// Runs `git` and returns its trimmed output, or the empty string for any
/// reason it did not produce one.
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
