use regex::RegexSet;
use ruma::api::appservice::Namespace;

/// The regular expressions of one namespace of a registration, compiled.
///
/// The two halves are kept apart rather than filtered at match time: a
/// namespace is checked against every locally created user, alias and room, so
/// the question asked most often — does anyone claim this exclusively — should
/// cost one pass over the patterns that could answer it.
///
/// Either half is `None` when the registration named no pattern of that kind,
/// which is the common case for one of the two and saves running an empty set.
#[derive(Clone, Debug, Default)]
pub struct NamespaceRegex {
    /// Patterns the appservice claims for itself alone.
    pub exclusive: Option<RegexSet>,

    /// Patterns it takes an interest in without excluding anyone else.
    pub non_exclusive: Option<RegexSet>,
}

impl NamespaceRegex {
    /// Whether the appservice has any claim on `haystack`, exclusive or not.
    #[inline]
    #[must_use]
    pub fn is_match(&self, haystack: &str) -> bool {
        self.is_exclusive_match(haystack) || matches(self.non_exclusive.as_ref(), haystack)
    }

    /// Whether the appservice claims `haystack` for itself alone, which is
    /// what bars anyone else from taking it.
    #[inline]
    #[must_use]
    pub fn is_exclusive_match(&self, haystack: &str) -> bool {
        matches(self.exclusive.as_ref(), haystack)
    }
}

/// Whether a set that may not exist matches. An absent set is one the
/// registration named no patterns for, and matches nothing.
#[inline]
fn matches(set: Option<&RegexSet>, haystack: &str) -> bool {
    set.is_some_and(|set| set.is_match(haystack))
}

/// Compiles the namespaces as they arrived in the registration.
///
/// Borrowed rather than owned: a registration is validated on the way in and
/// then kept whole beside the compiled form, so there is nothing here to take
/// ownership of.
impl TryFrom<&[Namespace]> for NamespaceRegex {
    type Error = regex::Error;

    fn try_from(namespaces: &[Namespace]) -> Result<Self, Self::Error> {
        Ok(Self {
            exclusive: compile(namespaces, true)?,
            non_exclusive: compile(namespaces, false)?,
        })
    }
}

/// The namespaces of one exclusivity as a set, or `None` where there are none.
fn compile(namespaces: &[Namespace], exclusive: bool) -> Result<Option<RegexSet>, regex::Error> {
    let patterns: Vec<&str> = namespaces
        .iter()
        .filter(|namespace| namespace.exclusive == exclusive)
        .map(|namespace| namespace.regex.as_str())
        .collect();

    if patterns.is_empty() {
        return Ok(None);
    }

    RegexSet::new(patterns).map(Some)
}
