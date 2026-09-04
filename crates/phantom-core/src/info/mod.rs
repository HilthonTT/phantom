//! Identity of this build.
//!
//! Nothing here is operator-configurable — it is baked in at compile time and
//! read back at runtime by the admin room, by the federation `User-Agent`, and
//! by log filtering that needs to tell our own events from a dependency's.

pub mod cargo;
pub mod rustc;
pub mod version;

pub use phantom_macros::rustc_flags_capture;

pub use self::version::{name, user_agent, version};

/// This crate's module root, i.e. `phantom_core`.
pub const MODULE_ROOT: &str = truncate_at(module_path!(), b':');

/// The prefix every crate in the workspace shares, i.e. `phantom`.
///
/// Log filtering compares a record's `module_path` against this to decide
/// whether an event came from us or from a dependency.
pub const CRATE_PREFIX: &str = truncate_at(MODULE_ROOT, b'_');

/// Everything in `s` before the first occurrence of `byte`, or all of `s` when
/// it does not occur.
///
/// A `const fn` rather than `const_str::split!` so the crate does not take a
/// mandatory proc-macro dependency for two constants. Only ASCII delimiters are
/// passed, so the split is always on a `char` boundary.
const fn truncate_at(s: &str, byte: u8) -> &str {
    debug_assert!(byte.is_ascii(), "delimiter must be ASCII to split safely");

    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == byte {
            break;
        }

        i = i.saturating_add(1);
    }

    s.split_at(i).0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_root_and_crate_prefix_name_this_workspace() {
        assert_eq!(MODULE_ROOT, "phantom_core");
        assert_eq!(CRATE_PREFIX, "phantom");
    }

    #[test]
    fn truncate_at_handles_missing_and_leading_delimiters() {
        assert_eq!(truncate_at("phantom_core::info", b':'), "phantom_core");
        assert_eq!(truncate_at("phantom", b'_'), "phantom", "no delimiter");
        assert_eq!(truncate_at("_phantom", b'_'), "", "leading delimiter");
        assert_eq!(truncate_at("", b'_'), "", "empty input");
    }
}
