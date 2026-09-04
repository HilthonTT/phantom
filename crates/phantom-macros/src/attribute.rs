//! Reading the arguments an attribute macro was invoked with.

use std::collections::HashMap;

use syn::{Expr, ExprLit, Generics, Lit, Meta, MetaNameValue, parse_str};

/// Collects `key = "value"` attribute arguments into a map, ignoring anything
/// that is not a name/value pair with a string literal.
pub(crate) fn get_simple_settings(args: &[Meta]) -> HashMap<String, String> {
    args.iter().fold(HashMap::new(), |mut map, arg| {
        let Meta::NameValue(MetaNameValue { path, value, .. }) = arg else {
            return map;
        };

        let Expr::Lit(ExprLit {
            lit: Lit::Str(str), ..
        }) = value
        else {
            return map;
        };

        if let Some(key) = path.segments.first().map(|segment| &segment.ident) {
            map.insert(key.to_string(), str.value());
        }

        map
    })
}

/// Parses a `<name> = "<generics>"` argument into [`syn::Generics`], defaulting
/// to an empty parameter list when absent.
pub(crate) fn get_named_generics(args: &[Meta], name: &str) -> crate::Result<Generics> {
    const DEFAULT: &str = "<>";

    parse_str::<Generics>(&get_named_string(args, name).unwrap_or_else(|| DEFAULT.to_owned()))
}

/// The value of a `<name> = "<value>"` argument, if present.
pub(crate) fn get_named_string(args: &[Meta], name: &str) -> Option<String> {
    args.iter().find_map(|arg| {
        let value = arg.require_name_value().ok()?;

        let Expr::Lit(ExprLit {
            lit: Lit::Str(str), ..
        }) = &value.value
        else {
            return None;
        };

        value.path.is_ident(name).then(|| str.value())
    })
}

/// This crate's name with the workspace prefix removed, i.e. `database` while
/// compiling `phantom-database`.
///
/// `None` outside a cargo build, where the macros that key a registry by crate
/// have nothing to key it by and expand to nothing instead.
pub(crate) fn get_crate_name() -> Option<String> {
    std::env::var("CARGO_CRATE_NAME")
        .ok()
        .map(|name| name.trim_start_matches("phantom_").to_owned())
}
