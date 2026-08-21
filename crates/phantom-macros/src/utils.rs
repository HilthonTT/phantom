//! Shared helpers for the macro implementations.

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

/// True when rustc was invoked to actually link a binary, as opposed to the
/// metadata-only passes `cargo check`, rust-analyzer and friends run.
///
/// The example file is only written on real builds so that editors re-checking
/// the crate on every keystroke do not keep rewriting it.
pub(crate) fn is_cargo_build() -> bool {
    // rustc accepts both `--emit=a,b` and `--emit a,b`, and each kind may carry
    // an `=path` suffix, so unpick all three shapes before looking for `link`.
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
