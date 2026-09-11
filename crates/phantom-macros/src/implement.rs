use proc_macro::TokenStream;
use quote::quote;
use syn::{Error, ItemFn, Meta, Path};

use crate::{Result, attribute::get_named_generics};

pub(crate) fn implement(item: ItemFn, args: &[Meta]) -> Result<TokenStream> {
    let generics = get_named_generics(args, "generics")?;
    let params = get_named_generics(args, "params")?;
    let receiver = get_receiver(args)?;

    let out = quote! {
        impl #generics #receiver #params {
            #item
        }
    };

    Ok(out.into())
}

fn get_receiver(args: &[Meta]) -> Result<Path> {
    let span = proc_macro2::Span::call_site();

    let receiver = args
        .first()
        .ok_or_else(|| Error::new(span, "missing required argument naming the receiver"))?;

    let Meta::Path(receiver) = receiver else {
        return Err(Error::new(
            span,
            "first argument is not a path to a receiver",
        ));
    };

    Ok(receiver.clone())
}
