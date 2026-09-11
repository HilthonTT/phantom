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
