use std::sync::LazyLock;

const BRANDING: &str = "phantom";

const SEMANTIC: &str = env!("CARGO_PKG_VERSION");

phantom_macros::git_semantic! {}
phantom_macros::git_commit! {}

static VERSION: LazyLock<String> = LazyLock::new(|| match build_id() {
    Some(extra) => format!("{}({extra})", semantic_prefix()),
    None => semantic().to_owned(),
});

static USER_AGENT: LazyLock<String> = LazyLock::new(|| format!("{BRANDING}/{}", version()));

#[inline]
#[must_use]
pub const fn name() -> &'static str {
    BRANDING
}

#[inline]
#[must_use]
pub fn semantic() -> &'static str {
    if GIT_SEMANTIC.is_empty() {
        SEMANTIC
    } else {
        GIT_SEMANTIC
    }
}

#[inline]
#[must_use]
pub fn commit() -> Option<&'static str> {
    (!GIT_COMMIT.is_empty()).then_some(GIT_COMMIT)
}

#[inline]
#[must_use]
pub fn version() -> &'static str {
    &VERSION
}

#[inline]
#[must_use]
pub fn user_agent() -> &'static str {
    &USER_AGENT
}

fn build_id() -> Option<&'static str> {
    option_env!("PHANTOM_VERSION_EXTRA")
        .filter(|extra| !extra.is_empty())
        .or_else(commit)
}

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

    #[test]
    fn semantic_is_never_empty() {
        assert!(!semantic().is_empty());
    }
}
