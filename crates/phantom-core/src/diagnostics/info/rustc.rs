use std::{
    collections::BTreeMap,
    mem::replace,
    sync::{Mutex, OnceLock},
};

phantom_macros::rustc_version! {}

pub static FLAGS: Mutex<BTreeMap<&str, &[&str]>> = Mutex::new(BTreeMap::new());

static FEATURES: OnceLock<Vec<&'static str>> = OnceLock::new();

#[inline]
pub fn features() -> &'static Vec<&'static str> {
    FEATURES.get_or_init(init_features)
}

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
