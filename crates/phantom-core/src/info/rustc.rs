//! What the compiler was told, read back at runtime.
//!
//! A build's cargo features are not otherwise visible to it: `cfg!` answers
//! only for features the source names, and a feature enabled by a dependency
//! resolving differently leaves no trace in the code. What does record them is
//! rustc's own command line, so each crate captures it at compile time with
//! [`phantom_macros::rustc_flags_capture`] and registers it here.
//!
//! Registration is a pre-main constructor per crate rather than a list this
//! module keeps, so a crate opts in by invoking the macro and nothing here has
//! to be kept in step with the workspace.

use std::{
    collections::BTreeMap,
    mem::replace,
    sync::{Mutex, OnceLock},
};

// The compiler that built this crate, captured the same way.
phantom_macros::rustc_version! {}

/// Each participating crate's compiler flags, by crate name.
///
/// Public only so that the constructor the macro expands to can reach it; it
/// is not somewhere to write from anywhere else.
pub static FLAGS: Mutex<BTreeMap<&str, &[&str]>> = Mutex::new(BTreeMap::new());

/// The cargo features enabled across the crates that registered.
static FEATURES: OnceLock<Vec<&'static str>> = OnceLock::new();

/// The cargo features this build actually has enabled.
///
/// Collected once, on first use: a crate that registers after that — which
/// cannot happen without dynamic loading — would not appear.
#[inline]
pub fn features() -> &'static Vec<&'static str> {
    FEATURES.get_or_init(init_features)
}

/// The compiler this was built with, as `rustc -V` reports it.
///
/// `None` for a build where the compiler could not be asked, which is the same
/// empty string the macro falls back to.
#[inline]
#[must_use]
pub fn version() -> Option<&'static str> {
    (!RUSTC_VERSION.is_empty()).then_some(RUSTC_VERSION)
}

fn init_features() -> Vec<&'static str> {
    let mut features = Vec::new();

    FLAGS
        .lock()
        .expect("the rustc flag registry is never held across a panic")
        .values()
        .for_each(|flags| append_features(&mut features, flags));

    features.sort_unstable();
    features.dedup();
    features
}

/// Picks the `--cfg feature="…"` pairs out of one crate's flags.
///
/// A feature is two arguments, not one: cargo passes `--cfg` and the value
/// after it, so the scan carries whether the previous argument opened one.
fn append_features(features: &mut Vec<&'static str>, flags: &[&'static str]) {
    let mut next_is_cfg = false;

    for flag in flags {
        let is_cfg = *flag == "--cfg";
        let is_feature = flag.starts_with("feature=");

        if replace(&mut next_is_cfg, is_cfg)
            && is_feature
            && let Some((_, feature)) = flag.split_once('=')
        {
            features.push(feature.trim_matches('"'));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The scan has to see `--cfg` and its value as a pair, and leave the
    /// other `--cfg`s — `debug_assertions`, the check-cfg names — alone.
    #[test]
    fn only_the_feature_cfgs_are_features() {
        let flags: &[&str] = &[
            "rustc",
            "--edition=2024",
            "--cfg",
            "feature=\"jemalloc\"",
            "--cfg",
            "debug_assertions",
            "--cfg",
            "feature=\"hardened_malloc\"",
            "-C",
            "opt-level=3",
        ];

        let mut features = Vec::new();
        append_features(&mut features, flags);

        assert_eq!(features, vec!["jemalloc", "hardened_malloc"]);
    }

    /// A `feature=` that is not preceded by `--cfg` is some other flag's
    /// value, not a feature.
    #[test]
    fn a_bare_feature_argument_is_not_one() {
        let flags: &[&str] = &["rustc", "feature=\"nope\"", "--cfg", "feature=\"yes\""];

        let mut features = Vec::new();
        append_features(&mut features, flags);

        assert_eq!(features, vec!["yes"]);
    }

    #[test]
    fn no_flags_are_no_features() {
        let mut features = Vec::new();
        append_features(&mut features, &[]);

        assert!(features.is_empty());
    }
}
