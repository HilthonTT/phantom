//! Keeping one async function's codegen out of its callers.

use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{Error, FnArg, ItemFn, Meta, ReturnType};

use crate::Result;

/// Splits an `async fn` into a boxed-future wrapper and a private body.
///
/// An `async fn` is a generic future type, so every caller monomorphises the
/// whole body into itself. On a call graph as deep as a request handler's that
/// compounds: build times grow, and the futures get large enough that holding
/// one across an await costs real stack. Type-erasing the future behind a
/// `Box<dyn Future>` cuts the chain — the body is compiled once, where it is
/// defined — at the cost of one allocation per call, which is why this is an
/// opt-in on the few functions worth it rather than a blanket policy.
///
/// Attributes below this one ride the body, so a `tracing` span still sees the
/// bindings it names; give such a span an explicit `name`, because the body is
/// the hidden inner function and would otherwise be logged under its mangled
/// name. Doc comments stay on the wrapper, which is the item callers see.
///
/// A single declared lifetime bounds the boxed future, and `'_` where there is
/// none or more than one. Borrows that must outlive the future together have to
/// be unified under one lifetime for that bound to hold.
pub(super) fn async_noinline(item: ItemFn, _args: &[Meta]) -> Result<TokenStream> {
    let ItemFn {
        attrs,
        vis,
        sig,
        block,
        ..
    } = item;

    if sig.asyncness.is_none() {
        return Err(Error::new(
            sig.ident.span(),
            "#[async_noinline] expects an async fn",
        ));
    }

    let output = match &sig.output {
        ReturnType::Default => quote!(()),
        ReturnType::Type(_, ty) => quote!(#ty),
    };

    let mut lifetimes = sig.generics.lifetimes().map(|def| &def.lifetime);
    let boxed_lifetime = match (lifetimes.next(), lifetimes.next()) {
        (Some(lifetime), None) => quote!(#lifetime),
        _ => quote!('_),
    };

    let (impl_generics, _, where_clause) = sig.generics.split_for_impl();

    let (doc_attrs, body_attrs): (Vec<_>, Vec<_>) =
        attrs.iter().partition(|attr| attr.path().is_ident("doc"));

    let name = &sig.ident;
    let inner_name = format_ident!("__{name}");
    let has_receiver = sig
        .inputs
        .iter()
        .any(|arg| matches!(arg, FnArg::Receiver(_)));

    // The wrapper's parameters are renamed: a pattern binding in the original
    // signature is destructuring, which the wrapper has nothing to destructure
    // yet, so each becomes one plain binding forwarded whole.
    let (wrapper_inputs, call_args): (Vec<_>, Vec<Option<_>>) = sig
        .inputs
        .iter()
        .enumerate()
        .map(|(i, arg)| match arg {
            FnArg::Receiver(receiver) => (quote!(#receiver), None),
            FnArg::Typed(arg) => {
                let binding = format_ident!("__arg{i}");
                let ty = &arg.ty;

                (quote!(#binding: #ty), Some(quote!(#binding)))
            }
        })
        .unzip();

    let call_args = call_args.into_iter().flatten();
    let self_prefix = has_receiver.then(|| quote!(self.));
    let call = quote!(#self_prefix #inner_name(#(#call_args),*));

    let inner_inputs = &sig.inputs;
    let inner_output = &sig.output;

    Ok(quote! {
        #(#doc_attrs)*
        #[inline(never)]
        #[must_use]
        #vis fn #name #impl_generics (#(#wrapper_inputs),*)
            -> ::std::pin::Pin<::std::boxed::Box<
                dyn ::std::future::Future<Output = #output> + Send + #boxed_lifetime
            >>
        #where_clause
        {
            ::std::boxed::Box::pin(#call)
        }

        #(#body_attrs)*
        async fn #inner_name #impl_generics (#inner_inputs) #inner_output
        #where_clause
        #block
    }
    .into())
}
