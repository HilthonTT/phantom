//! Matching rules a URL parser does not expose.

/// Whether `hostname` is `domain` or sits beneath it.
///
/// Case-insensitive over ASCII, which is what DNS comparison is. A leading dot
/// on `domain` is accepted and ignored, so the `.example.com` an operator is
/// used to writing in a no-proxy list means the same as `example.com`.
///
/// The suffix has to fall on a label boundary. Without that check
/// `notexample.com` matches `example.com`, which is how a proxy exemption or an
/// allowlist ends up covering a domain somebody else registered.
///
/// The root, spelled `.`, matches only a fully qualified name — one written
/// with the trailing dot — because that is the only form that names the root
/// explicitly.
#[must_use]
pub fn hostname_matches_domain(hostname: &str, domain: &str) -> bool {
    if domain == "." {
        return hostname.ends_with('.');
    }

    let domain = domain.strip_prefix('.').unwrap_or(domain);

    if domain.is_empty() {
        return false;
    }

    if hostname.eq_ignore_ascii_case(domain) {
        return true;
    }

    // The byte before the suffix has to be the label separator: `hostname` is
    // `<something>.<domain>` or it is not a match at all.
    let Some(separator) = hostname
        .len()
        .checked_sub(domain.len())
        .and_then(|start| start.checked_sub(1))
    else {
        return false;
    };

    hostname.as_bytes().get(separator) == Some(&b'.')
        && hostname
            .get(separator.saturating_add(1)..)
            .is_some_and(|suffix| suffix.eq_ignore_ascii_case(domain))
}

#[cfg(test)]
mod tests {
    use super::hostname_matches_domain;

    #[test]
    fn a_domain_matches_itself_and_its_subdomains() {
        assert!(hostname_matches_domain("example.com", "example.com"));
        assert!(hostname_matches_domain("matrix.example.com", "example.com"));
        assert!(hostname_matches_domain("a.b.c.example.com", "example.com"));
    }

    /// The check that stops an exemption for one domain covering another.
    #[test]
    fn a_suffix_only_matches_on_a_label_boundary() {
        assert!(!hostname_matches_domain("notexample.com", "example.com"));
        assert!(!hostname_matches_domain("evilexample.com", "example.com"));
        assert!(!hostname_matches_domain(
            "example.com.evil.net",
            "example.com"
        ));
    }

    #[test]
    fn matching_ignores_ascii_case() {
        assert!(hostname_matches_domain("EXAMPLE.COM", "example.com"));
        assert!(hostname_matches_domain("Matrix.Example.Com", "EXAMPLE.com"));
    }

    /// A no-proxy list is conventionally written with the leading dot.
    #[test]
    fn a_leading_dot_on_the_domain_is_ignored() {
        assert!(hostname_matches_domain("example.com", ".example.com"));
        assert!(hostname_matches_domain(
            "matrix.example.com",
            ".example.com"
        ));
    }

    #[test]
    fn a_shorter_hostname_never_matches() {
        assert!(!hostname_matches_domain("com", "example.com"));
        assert!(!hostname_matches_domain("", "example.com"));
    }

    /// An empty domain would otherwise match everything, which is not what an
    /// empty entry in a list is meant to say.
    #[test]
    fn an_empty_domain_matches_nothing() {
        assert!(!hostname_matches_domain("example.com", ""));
        assert!(!hostname_matches_domain("example.com", "."));
    }

    #[test]
    fn the_root_matches_only_a_fully_qualified_name() {
        assert!(hostname_matches_domain("example.com.", "."));
        assert!(!hostname_matches_domain("example.com", "."));
    }
}
