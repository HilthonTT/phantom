//! Embedding a crate's `Cargo.toml` in the binary.
//!
//! The manifests answer questions an operator asks a running server — which
//! features exist, what it was built against — and the source tree they come
//! from is not there to be read at runtime. So the text is baked in at compile
//! time instead of opened later.

use std::{fs::read_to_string, path::PathBuf};

use proc_macro::{Span, TokenStream};
use quote::quote;
use syn::{Error, ItemConst, Meta};

use crate::{Result, attribute::get_named_string};

/// Replaces the annotated `const` with the text of a workspace manifest.
///
/// The annotated item's own value is discarded — it is only there to name the
/// constant and give it a type — so it is written as `()`:
///
/// ```ignore
/// #[cargo_manifest]
/// const WORKSPACE_MANIFEST: &'static str = ();
/// #[cargo_manifest(crate = "database")]
/// const DATABASE_MANIFEST: &'static str = ();
/// ```
///
/// A manifest that cannot be read becomes the empty string rather than a
/// compile error: the build itself is still valid, and what is lost is one
/// answer to an admin query.
pub(super) fn manifest(item: ItemConst, args: &[Meta]) -> Result<TokenStream> {
    let member = get_named_string(args, "crate");
    let path = manifest_path(member.as_deref())?;
    let manifest = read_to_string(&path).unwrap_or_default();

    let name = item.ident;
    let val = manifest.as_str();

    Ok(quote! {
        const #name: &'static str = #val;
    }
    .into())
}

/// The manifest of the named workspace member, or of the workspace itself.
///
/// Walked from this crate's own directory rather than from the current working
/// directory, which during a build is wherever cargo was invoked.
fn manifest_path(member: Option<&str>) -> Result<PathBuf> {
    let Some(path) = option_env!("CARGO_MANIFEST_DIR") else {
        return Err(Error::new(
            Span::call_site().into(),
            "missing CARGO_MANIFEST_DIR in environment",
        ));
    };

    // <root>/crates/phantom-macros -> <root>/crates
    let mut path = PathBuf::from(path);
    path.pop();

    match member {
        // <root>/crates/phantom-<member>
        Some(member) => path.push(format!("phantom-{member}")),
        // <root>/crates -> <root>
        None => {
            path.pop();
        }
    }

    path.push("Cargo.toml");

    Ok(path)
}
