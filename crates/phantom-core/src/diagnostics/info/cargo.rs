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

pub fn features() -> &'static Vec<String> {
    FEATURES.get_or_init(|| init_features().expect("embedded manifests parse"))
}

pub fn dependencies() -> &'static DepsSet {
    DEPENDENCIES.get_or_init(|| init_dependencies().expect("embedded manifests parse"))
}

#[must_use]
pub fn dependency_names() -> Vec<&'static str> {
    dependencies().keys().map(String::as_str).collect()
}

fn parse(manifest: &str) -> Result<Manifest> {
    Manifest::from_str(manifest).map_err(|e| err!("Failed to parse an embedded manifest: {e}"))
}

fn init_features() -> Result<Vec<String>> {
    let mut features = Vec::new();

    for manifest in MANIFESTS {
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
