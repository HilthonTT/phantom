//! The one place the server's version string is assembled.
//!
//! Set `PHANTOM_VERSION_EXTRA` at build time to append a build identifier —
//! typically a git commit hash — after the SemVer version, e.g.
//! `0.1.0 (a1b2c3d)`. An empty value is treated as unset so a CI job can
//! export the variable unconditionally.

use std::sync::LazyLock;

/// The name the server calls itself in federation and in the admin room.
const BRANDING: &str = "phantom";

/// The SemVer version of the workspace, from Cargo.
const SEMANTIC: &str = env!("CARGO_PKG_VERSION");

static VERSION: LazyLock<String> = LazyLock::new(|| match option_env!("PHANTOM_VERSION_EXTRA") {
    Some(extra) if !extra.is_empty() => format!("{SEMANTIC} ({extra})"),
    _ => SEMANTIC.to_owned(),
});

static USER_AGENT: LazyLock<String> = LazyLock::new(|| format!("{BRANDING}/{}", version()));

/// The server's branding, without a version.
#[inline]
#[must_use]
pub const fn name() -> &'static str {
    BRANDING
}

/// The full version, including any build identifier.
#[inline]
#[must_use]
pub fn version() -> &'static str {
    &VERSION
}

/// The `User-Agent` phantom sends on outbound federation requests.
#[inline]
#[must_use]
pub fn user_agent() -> &'static str {
    &USER_AGENT
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_starts_with_the_cargo_version() {
        assert!(version().starts_with(SEMANTIC), "{}", version());
    }

    #[test]
    fn user_agent_is_name_slash_version() {
        assert_eq!(user_agent(), format!("{}/{}", name(), version()));
    }
}
