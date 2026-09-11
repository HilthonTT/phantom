pub mod cargo;
pub mod rustc;
pub mod version;

pub use phantom_macros::rustc_flags_capture;

pub use self::version::{name, user_agent, version};

pub const MODULE_ROOT: &str = truncate_at(module_path!(), b':');

pub const CRATE_PREFIX: &str = truncate_at(MODULE_ROOT, b'_');

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
