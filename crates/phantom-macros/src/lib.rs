//! Procedural macros for the phantom workspace.
//!
//! Proc macros must live in their own `proc-macro = true` crate, so this sits
//! next to `phantom-core` rather than inside it.

mod async_noinline;
mod attribute;
mod cargo;
mod config;
mod debug;
mod git;
mod implement;
mod rustc;

use proc_macro::TokenStream;
use syn::{
    Error, ItemConst, ItemFn, ItemStruct, Meta,
    parse::{Parse, Parser},
    parse_macro_input,
};

pub(crate) type Result<T> = std::result::Result<T, Error>;

/// Emits a `Display` impl summarising the annotated config struct, and — during
/// a real `cargo build` — writes a documented example TOML file derived from
/// the struct's fields and doc comments.
///
/// ```ignore
/// #[config_example_generator(
///     filename = "phantom-example.toml",
///     section = "global",
///     header = "### phantom configuration\n",
///     ignore = "catchall"
/// )]
/// pub struct Config { /* ... */ }
/// ```
#[proc_macro_attribute]
pub fn config_example_generator(args: TokenStream, input: TokenStream) -> TokenStream {
    attribute_macro::<ItemStruct, _>(args, input, config::example_generator)
}

/// Parses the attribute's arguments and item, then hands both to `func`,
/// turning any [`syn::Error`] into a `compile_error!` in place.
fn attribute_macro<I, F>(args: TokenStream, input: TokenStream, func: F) -> TokenStream
where
    F: Fn(I, &[Meta]) -> Result<TokenStream>,
    I: Parse,
{
    let item = parse_macro_input!(input as I);

    syn::punctuated::Punctuated::<Meta, syn::Token![,]>::parse_terminated
        .parse(args)
        .map(|args| args.iter().cloned().collect::<Vec<_>>())
        .and_then(|ref args| func(item, args))
        .unwrap_or_else(|error| error.to_compile_error().into())
}

#[proc_macro_attribute]
pub fn recursion_depth(args: TokenStream, input: TokenStream) -> TokenStream {
    attribute_macro::<ItemStruct, _>(args, input, debug::recursion_depth)
}

/// Wraps the annotated function in an `impl` block for the named receiver.
///
/// ```ignore
/// #[implement(Manager)]
/// fn load(&self) -> &Config { /* ... */ }
/// ```
#[proc_macro_attribute]
pub fn implement(args: TokenStream, input: TokenStream) -> TokenStream {
    attribute_macro::<ItemFn, _>(args, input, implement::implement)
}

/// Replaces the annotated `const` with the text of a workspace `Cargo.toml`.
///
/// ```ignore
/// #[cargo_manifest]
/// const WORKSPACE_MANIFEST: &'static str = ();
/// #[cargo_manifest(crate = "database")]
/// const DATABASE_MANIFEST: &'static str = ();
/// ```
#[proc_macro_attribute]
pub fn cargo_manifest(args: TokenStream, input: TokenStream) -> TokenStream {
    attribute_macro::<ItemConst, _>(args, input, cargo::manifest)
}

/// Splits an `async fn` into an `#[inline(never)]` wrapper returning a boxed
/// future and a private body, so the body is compiled once rather than into
/// every caller.
#[proc_macro_attribute]
pub fn async_noinline(args: TokenStream, input: TokenStream) -> TokenStream {
    attribute_macro::<ItemFn, _>(args, input, async_noinline::async_noinline)
}

/// Defines `RUSTC_FLAGS` for this crate and registers it with
/// `phantom_core::info::rustc`, which is how a build reports the cargo
/// features it was actually compiled with.
#[proc_macro]
pub fn rustc_flags_capture(args: TokenStream) -> TokenStream {
    rustc::flags_capture(args)
}

/// Defines `RUSTC_VERSION`, the output of `rustc -V` for this build.
#[proc_macro]
pub fn rustc_version(args: TokenStream) -> TokenStream {
    rustc::version(args)
}

/// Defines `GIT_SEMANTIC`, the version of the nearest tag this was built from.
#[proc_macro]
pub fn git_semantic(args: TokenStream) -> TokenStream {
    git::semantic(args)
}

/// Defines `GIT_COMMIT`, the commit this was built from.
#[proc_macro]
pub fn git_commit(args: TokenStream) -> TokenStream {
    git::commit(args)
}
