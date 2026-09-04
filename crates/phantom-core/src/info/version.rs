//! The one place the server's version string is assembled.
//!
//! Three sources, in order of preference. The nearest git tag is the most
//! specific thing a build knows about itself, so it wins where there is one;
//! `CARGO_PKG_VERSION` is the fallback for a build from a tarball, where there
//! is no repository to ask. The build identifier after it — the commit, or
//! whatever `PHANTOM_VERSION_EXTRA` was set to — says which build of that
//! version this is.
//!
//! All of it is resolved at compile time by [`phantom_macros::git_semantic`]
//! and [`phantom_macros::git_commit`], because the checkout is not there to be
//! read at runtime.

use std::sync::LazyLock;

/// The name the server calls itself in federation and in the admin room.
const BRANDING: &str = "phantom";

/// The SemVer version of the workspace, from Cargo.
const SEMANTIC: &str = env!("CARGO_PKG_VERSION");

// `GIT_SEMANTIC` and `GIT_COMMIT`, both empty where git could not be asked.
phantom_macros::git_semantic! {}
phantom_macros::git_commit! {}

static VERSION: LazyLock<String> = LazyLock::new(|| match build_id() {
    Some(extra) => format!("{}({extra})", semantic_prefix()),
    None => semantic().to_owned(),
});

static USER_AGENT: LazyLock<String> = LazyLock::new(|| format!("{BRANDING}/{}", version()));

/// The server's branding, without a version.
#[inline]
#[must_use]
pub const fn name() -> &'static str {
    BRANDING
}

/// The SemVer version, without any build identifier.
///
/// The tag is preferred over Cargo's version because a workspace whose version
/// has been bumped but not yet tagged is still, as far as anyone deploying it
/// is concerned, the last release plus some commits.
#[inline]
#[must_use]
pub fn semantic() -> &'static str {
    if GIT_SEMANTIC.is_empty() {
        SEMANTIC
    } else {
        GIT_SEMANTIC
    }
}

/// The commit this was built from, `-dirty` where the tree was not clean.
///
/// `None` for a build with no repository to read, such as one from a release
/// tarball.
#[inline]
#[must_use]
pub fn commit() -> Option<&'static str> {
    (!GIT_COMMIT.is_empty()).then_some(GIT_COMMIT)
}

/// The full version, including any build identifier: `0.1.0 (a1b2c3d)`.
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

/// Which build of [`semantic`] this is, if anything says.
///
/// `PHANTOM_VERSION_EXTRA` overrides the commit so that a packager can stamp
/// its own identifier — a distribution revision, a CI build number — on a
/// build made from a checkout. An empty value is treated as unset, so a CI job
/// may export it unconditionally.
fn build_id() -> Option<&'static str> {
    option_env!("PHANTOM_VERSION_EXTRA")
        .filter(|extra| !extra.is_empty())
        .or_else(commit)
}

/// [`semantic`] with the trailing space a build identifier is appended after.
fn semantic_prefix() -> String {
    format!("{} ", semantic())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_version_starts_with_the_semantic_one() {
        assert!(version().starts_with(semantic()), "{}", version());
    }

    /// Without a repository or an override there is nothing to put after the
    /// version, and it stands alone rather than trailing an empty bracket.
    #[test]
    fn a_build_identifier_is_bracketed_or_absent() {
        match build_id() {
            Some(extra) => assert_eq!(version(), format!("{} ({extra})", semantic())),
            None => assert_eq!(version(), semantic()),
        }
    }

    #[test]
    fn user_agent_is_name_slash_version() {
        assert_eq!(user_agent(), format!("{}/{}", name(), version()));
    }

    /// A tag is only preferred where there is one; a tarball build still has a
    /// version.
    #[test]
    fn semantic_is_never_empty() {
        assert!(!semantic().is_empty());
    }
}
