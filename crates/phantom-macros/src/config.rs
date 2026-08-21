//! Implementation of `#[config_example_generator]`.
//!
//! The attribute walks the annotated struct's named fields and turns each one
//! into a commented-out entry in a generated TOML file, using the field's doc
//! comment as its documentation and its `#[serde(default = "...")]` as the
//! shown value. It additionally emits a `Display` impl rendering the live
//! config as a markdown table.
//!
//! Doc comments may carry directives on their own line:
//!
//! - `default: <text>` overrides the value shown in the example file.
//! - `display: hidden` omits the field from the `Display` table.
//! - `display: sensitive` masks the field's value in the `Display` table.
//!
//! Directive lines are stripped from the documentation written to the file.

use std::{collections::HashSet, fmt::Write as _, fs::OpenOptions, io::Write as _};

use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::{ToTokens, quote};
use syn::{
    Error, Expr, ExprLit, Field, Fields, FieldsNamed, ItemStruct, Lit, Meta, MetaList,
    MetaNameValue, parse::Parser, punctuated::Punctuated, spanned::Spanned,
};

use crate::{
    Result,
    utils::{get_simple_settings, is_cargo_build, is_cargo_test},
};

const UNDOCUMENTED: &str = "# This item is undocumented. Please contribute documentation for it.";

/// Doc comment directives, stripped from the generated documentation.
const DIRECTIVES: &[&str] = &["default", "display"];

pub(crate) fn example_generator(input: ItemStruct, args: &[Meta]) -> Result<TokenStream> {
    // Only a linking build writes the file; `cargo check` and rust-analyzer
    // expand this macro constantly and must not touch the tree.
    let write = is_cargo_build() && !is_cargo_test();
    let generated = generate_example(&input, args, write)?;

    Ok([input.to_token_stream(), generated]
        .into_iter()
        .collect::<TokenStream2>()
        .into())
}

fn generate_example(input: &ItemStruct, args: &[Meta], write: bool) -> Result<TokenStream2> {
    let settings = get_simple_settings(args);
    let span = args.first().map_or_else(Span::call_site, Spanned::span);

    let required = |key: &str| {
        settings
            .get(key)
            .ok_or_else(|| Error::new(span, format!("missing required `{key}` argument")))
    };

    let section = required("section")?;
    let filename = required("filename")?;

    let undocumented = settings
        .get("undocumented")
        .map_or(UNDOCUMENTED, String::as_str);

    let ignore: HashSet<&str> = settings
        .get("ignore")
        .map_or("", String::as_str)
        .split_whitespace()
        .collect();

    // The root section truncates the file; every other section appends to it,
    // so section structs must be expanded after the root one.
    let is_root = section == "global";

    let mut file = write
        .then(|| {
            OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(is_root)
                .append(!is_root)
                .open(filename)
                .map_err(|error| {
                    let msg = format!("failed to open {filename} for config generation: {error}");
                    Error::new(Span::call_site(), msg)
                })
        })
        .transpose()?;

    if let Some(file) = file.as_mut() {
        if let Some(header) = settings.get("header") {
            write_to(file, header);
        }

        write_to(file, &format!("\n[{section}]\n"));
    }

    let mut summary: Vec<TokenStream2> = Vec::new();

    if let Fields::Named(FieldsNamed { named, .. }) = &input.fields {
        for field in named {
            let Some(ident) = &field.ident else {
                continue;
            };

            if ignore.contains(ident.to_string().as_str()) {
                continue;
            }

            // Stripping directive lines can leave the comment ending in a bare
            // `#`; keep exactly one such separator line before the key.
            let doc = get_doc_comment(field).unwrap_or_else(|| format!("{undocumented}\n"));
            let doc = doc.trim_end();
            let doc = if doc.ends_with('#') {
                format!("{doc}\n")
            } else {
                format!("{doc}\n#\n")
            };

            let default = get_directive(field, "default")
                .or_else(|| get_serde_default(field))
                .map(|default| format!(" {default}"))
                .unwrap_or_default();

            if let Some(file) = file.as_mut() {
                write_to(file, &format!("\n{doc}#{ident} ={default}\n"));
            }

            let display = get_directive(field, "display").unwrap_or_default();
            let mut directives = display.split_whitespace();

            if directives.clone().any(|directive| directive == "hidden") {
                continue;
            }

            let value = if directives.any(|directive| directive == "sensitive") {
                quote! { "***********" }
            } else {
                quote! { format_args!("{:?}", self.#ident) }
            };

            let name = ident.to_string();
            summary.push(quote! {
                writeln!(out, "| {} | {} |", #name, #value)?;
            });
        }
    }

    if let Some(file) = file.as_mut()
        && let Some(footer) = settings.get("footer")
    {
        write_to(file, footer);
    }

    let struct_name = &input.ident;

    Ok(quote! {
        impl std::fmt::Display for #struct_name {
            fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                writeln!(out, "| name | value |")?;
                writeln!(out, "| :--- | :---  |")?;
                #( #summary )*
                Ok(())
            }
        }
    })
}

fn write_to(file: &mut std::fs::File, text: &str) {
    file.write_all(text.as_bytes())
        .expect("written to config file");
}

/// Reads the value a `#[serde(default = "...")]` would produce, for the handful
/// of shapes that have an obvious TOML spelling.
fn get_serde_default(field: &Field) -> Option<String> {
    for attr in &field.attrs {
        let Meta::List(MetaList { path, tokens, .. }) = &attr.meta else {
            continue;
        };

        if path.segments.first().is_none_or(|s| s.ident != "serde") {
            continue;
        }

        let args = Punctuated::<Meta, syn::Token![,]>::parse_terminated
            .parse(tokens.clone().into())
            .ok()?;

        match args.first()? {
            // `#[serde(default)]` — the type's own `Default`.
            Meta::Path(_) => return Some("false".to_owned()),
            Meta::NameValue(MetaNameValue {
                value:
                    Expr::Lit(ExprLit {
                        lit: Lit::Str(str), ..
                    }),
                ..
            }) => {
                return match str.value().as_str() {
                    "true_fn" => Some("true".to_owned()),
                    "Vec::new" | "HashSet::new" | "BTreeSet::new" => Some("[]".to_owned()),
                    // Anything else needs an explicit `default:` doc directive.
                    _ => None,
                };
            }
            _ => return None,
        }
    }

    None
}

/// The field's doc comment as TOML comment lines, with directive lines removed.
fn get_doc_comment(field: &Field) -> Option<String> {
    let comment = get_doc_comment_full(field)?;

    let out = comment
        .lines()
        .filter(|line| !is_directive(line))
        .fold(String::new(), |full, line| full + "#" + line + "\n");

    (!out.trim().is_empty()).then_some(out)
}

/// The value of a `<label>: <value>` directive line in the field's doc comment.
fn get_directive(field: &Field, label: &str) -> Option<String> {
    get_doc_comment_full(field)?
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with(label) && line[label.len()..].starts_with(':'))
        .find_map(|line| {
            line.split_once(':')
                .map(|(_, value)| value.trim().to_owned())
        })
        .filter(|value| !value.is_empty())
}

fn is_directive(line: &str) -> bool {
    let line = line.trim();

    DIRECTIVES
        .iter()
        .any(|label| line.starts_with(label) && line[label.len()..].starts_with(':'))
}

fn get_doc_comment_full(field: &Field) -> Option<String> {
    let mut out = String::new();

    for attr in &field.attrs {
        let Meta::NameValue(MetaNameValue { path, value, .. }) = &attr.meta else {
            continue;
        };

        if path.segments.first().is_none_or(|s| s.ident != "doc") {
            continue;
        }

        let Expr::Lit(ExprLit {
            lit: Lit::Str(token),
            ..
        }) = value
        else {
            continue;
        };

        writeln!(&mut out, "{}", token.value()).expect("wrote to output string buffer");
    }

    (!out.is_empty()).then_some(out)
}
