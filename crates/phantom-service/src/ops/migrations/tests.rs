use std::collections::HashSet;

use regex::RegexSet;

use super::{STEPS, matched_patterns};

#[test]
fn step_markers_are_unique() {
    let mut seen = HashSet::new();

    for step in STEPS {
        assert!(
            seen.insert(step.marker),
            "duplicate migration marker {}",
            step.marker
        );
    }
}

#[test]
fn matching_patterns_are_listed_in_order() {
    let set = RegexSet::new(["^adm", "bad", "min$"]).expect("valid patterns");

    assert_eq!(matched_patterns(&set, "admin"), "^adm, min$");
}

#[test]
fn no_match_lists_nothing() {
    let set = RegexSet::new(["^adm"]).expect("valid pattern");

    assert!(matched_patterns(&set, "alice").is_empty());
}
