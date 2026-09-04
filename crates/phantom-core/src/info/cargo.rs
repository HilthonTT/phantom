//! The workspace manifests, as they were at build time.
//!
//! What an operator asks — which features this build could have had, what it
//! depends on — is answered from `Cargo.toml`, and the source tree is not
//! there to be read at runtime. So the manifests are baked into the binary by
//! [`phantom_macros::cargo_manifest`] and parsed here on first use.
//!
//! This is the *possible* feature set. For the features a build actually has
//! enabled, see [`rustc::features`](super::rustc::features).

use std::sync::OnceLock;

use cargo_toml::{DepsSet, Manifest};
use phantom_macros::cargo_manifest;

use crate::{Result, err};

#[cargo_manifest]
const WORKSPACE_MANIFEST: &'static str = ();
#[cargo_manifest(crate = "core")]
const CORE_MANIFEST: &'static str = ();
#[cargo_manifest(crate = "database")]
const DATABASE_MANIFEST: &'static str = ();
#[cargo_manifest(crate = "macros")]
const MACROS_MANIFEST: &'static str = ();
#[cargo_manifest(crate = "server")]
const SERVER_MANIFEST: &'static str = ();
#[cargo_manifest(crate = "service")]
const SERVICE_MANIFEST: &'static str = ();

/// Every manifest embedded above, so a new crate is added in one place.
const MANIFESTS: &[&str] = &[
    WORKSPACE_MANIFEST,
    CORE_MANIFEST,
    DATABASE_MANIFEST,
    MACROS_MANIFEST,
    SERVER_MANIFEST,
    SERVICE_MANIFEST,
];

static FEATURES: OnceLock<Vec<String>> = OnceLock::new();
static DEPENDENCIES: OnceLock<DepsSet> = OnceLock::new();

/// Every feature any crate in the workspace declares, sorted and deduplicated.
///
/// # Panics
///
/// Panics where an embedded manifest does not parse, which would mean the
/// build read a `Cargo.toml` cargo itself had already rejected.
pub fn features() -> &'static Vec<String> {
    FEATURES.get_or_init(|| init_features().expect("embedded manifests parse"))
}

/// The workspace's shared dependency table.
///
/// # Panics
///
/// Panics where the workspace manifest does not parse or has no `[workspace]`
/// section, neither of which is reachable from a manifest cargo accepted.
pub fn dependencies() -> &'static DepsSet {
    DEPENDENCIES.get_or_init(|| init_dependencies().expect("embedded manifests parse"))
}

/// The names in [`dependencies`], which is what a listing wants.
#[must_use]
pub fn dependency_names() -> Vec<&'static str> {
    dependencies().keys().map(String::as_str).collect()
}

/// `cargo_toml`'s error is not one of ours, so it is restated as one.
fn parse(manifest: &str) -> Result<Manifest> {
    Manifest::from_str(manifest).map_err(|e| err!("Failed to parse an embedded manifest: {e}"))
}

fn init_features() -> Result<Vec<String>> {
    let mut features = Vec::new();

    for manifest in MANIFESTS {
        // A manifest that could not be read at build time is embedded as the
        // empty string, which parses as a manifest declaring nothing.
        features.extend(parse(manifest)?.features.into_keys());
    }

    features.sort();
    features.dedup();

    Ok(features)
}

fn init_dependencies() -> Result<DepsSet> {
    let manifest = parse(WORKSPACE_MANIFEST)?;

    Ok(manifest
        .workspace
        .map(|workspace| workspace.dependencies)
        .unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The manifests are embedded by path, so a move of the crate directories
    /// shows up here as an empty parse rather than a build failure.
    #[test]
    fn the_workspace_manifest_was_found() {
        assert!(
            WORKSPACE_MANIFEST.contains("[workspace]"),
            "workspace manifest is empty or not the workspace's"
        );
        assert!(
            CORE_MANIFEST.contains("phantom-core"),
            "core manifest is empty or not core's"
        );
    }

    #[test]
    fn the_declared_features_include_this_crates_own() {
        assert!(features().iter().any(|feature| feature == "jemalloc"));
    }

    #[test]
    fn the_workspace_dependencies_are_listed() {
        assert!(dependency_names().contains(&"phantom-core"));
    }
}
