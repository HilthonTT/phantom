use regex::RegexSet;
use ruma::api::appservice::Namespace;

#[derive(Clone, Debug, Default)]
pub struct NamespaceRegex {
    pub exclusive: Option<RegexSet>,

    pub non_exclusive: Option<RegexSet>,
}

impl NamespaceRegex {
    #[inline]
    #[must_use]
    pub fn is_match(&self, haystack: &str) -> bool {
        self.is_exclusive_match(haystack) || matches(self.non_exclusive.as_ref(), haystack)
    }

    #[inline]
    #[must_use]
    pub fn is_exclusive_match(&self, haystack: &str) -> bool {
        matches(self.exclusive.as_ref(), haystack)
    }
}

#[inline]
fn matches(set: Option<&RegexSet>, haystack: &str) -> bool {
    set.is_some_and(|set| set.is_match(haystack))
}

impl TryFrom<&[Namespace]> for NamespaceRegex {
    type Error = regex::Error;

    fn try_from(namespaces: &[Namespace]) -> Result<Self, Self::Error> {
        Ok(Self {
            exclusive: compile(namespaces, true)?,
            non_exclusive: compile(namespaces, false)?,
        })
    }
}

fn compile(namespaces: &[Namespace], exclusive: bool) -> Result<Option<RegexSet>, regex::Error> {
    let patterns: Vec<String> = namespaces
        .iter()
        .filter(|namespace| namespace.exclusive == exclusive)
        .map(|namespace| format!("^(?:{})", namespace.regex))
        .collect();

    if patterns.is_empty() {
        return Ok(None);
    }

    RegexSet::new(patterns).map(Some)
}
