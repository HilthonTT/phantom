//! Procedural macros for the phantom workspace.
//!
//! Proc macros must live in their own `proc-macro = true` crate, so this sits
//! next to `phantom-core` rather than inside it.

mod config;
mod debug;
mod implement;
mod utils;

use proc_macro::TokenStream;
use syn::{
    Error, ItemFn, ItemStruct, Meta,
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
