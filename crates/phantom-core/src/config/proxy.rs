use reqwest::{Proxy, Url};
use serde::Deserialize;

use crate::Result;

/// ## Examples:
/// - No proxy (default):
/// ```toml
/// proxy ="none"
/// ```
/// - Global proxy
/// ```toml
/// [global.proxy]
/// global = { url = "socks5h://localhost:9050" }
/// ```
/// - Proxy some domains
/// ```toml
/// [global.proxy]
/// [[global.proxy.by_domain]]
/// url = "socks5h://localhost:9050"
/// include = ["*.onion", "matrix.myspecial.onion"]
/// exclude = ["*.myspecial.onion"]
/// ```
/// ## Include vs. Exclude
/// If include is an empty list, it is assumed to be `["*"]`.
///
/// If a domain matches both the exclude and include list, the proxy will only
/// be used if it was included because of a more specific rule than it was
/// excluded. In the above example, the proxy would be used for
/// `ordinary.onion`, `matrix.myspecial.onion`, but not `hello.myspecial.onion`.
#[derive(Clone, Default, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProxyConfig {
    #[default]
    None,
    Global {
        #[serde(deserialize_with = "crate::json::deserialize_from_str")]
        url: Url,
    },
    ByDomain(Vec<PartialProxyConfig>),
}
impl ProxyConfig {
    pub fn to_proxy(&self) -> Result<Option<Proxy>> {
        Ok(match self.clone() {
            Self::None => None,
            Self::Global { url } => Some(Proxy::all(url)?),
            Self::ByDomain(proxies) => Some(Proxy::custom(move |url| {
                proxies.iter().find_map(|proxy| proxy.for_url(url)).cloned()
            })),
        })
    }
}

/// Proxy schemes that resolve the destination hostname at the proxy rather
/// than in this process. `socks5h` and `socks4a` are the resolving spellings
/// of their families; the plain `socks5` and `socks4` are not.
const RESOLVES_REMOTELY: [&str; 4] = ["http", "https", "socks4a", "socks5h"];

impl ProxyConfig {
    /// Whether `url` names a configured proxy endpoint that this process
    /// would resolve itself.
    ///
    /// A request aimed at the proxy's own hostname is indistinguishable, at
    /// the resolver, from the connection to the proxy — so a URL preview
    /// pointed at it would be handed the exemption the proxy endpoint has.
    /// Where the proxy resolves the destination instead, no local lookup
    /// happens and there is nothing to alias.
    #[must_use]
    pub fn resolver_alias(&self, url: &Url) -> bool {
        let Some(host) = url.host_str() else {
            return false;
        };

        let names_a_proxy = self
            .endpoints()
            .filter_map(|endpoint| endpoint.host_str())
            .any(|endpoint| endpoint.eq_ignore_ascii_case(host));

        names_a_proxy
            && self
                .proxy_for(url)
                .is_none_or(|proxy| !RESOLVES_REMOTELY.contains(&proxy.scheme()))
    }

    /// Every proxy endpoint this configuration names, whichever rule reaches
    /// it.
    fn endpoints(&self) -> impl Iterator<Item = &Url> {
        let (global, by_domain): (Option<&Url>, &[PartialProxyConfig]) = match self {
            Self::None => (None, &[]),
            Self::Global { url } => (Some(url), &[]),
            Self::ByDomain(proxies) => (None, proxies.as_slice()),
        };

        global
            .into_iter()
            .chain(by_domain.iter().map(|proxy| &proxy.url))
    }

    /// Whether a request for `url` would be carried by a proxy.
    #[must_use]
    pub fn intercepts(&self, url: &Url) -> bool {
        self.proxy_for(url).is_some()
    }

    /// The proxy `url` would be carried by, if any.
    fn proxy_for(&self, url: &Url) -> Option<&Url> {
        match self {
            Self::None => None,
            Self::Global { url: proxy } => Some(proxy),
            Self::ByDomain(proxies) => proxies.iter().find_map(|proxy| proxy.for_url(url)),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct PartialProxyConfig {
    #[serde(deserialize_with = "crate::json::deserialize_from_str")]
    url: Url,
    #[serde(default)]
    include: Vec<WildCardedDomain>,
    #[serde(default)]
    exclude: Vec<WildCardedDomain>,
}
impl PartialProxyConfig {
    #[must_use]
    pub fn for_url(&self, url: &Url) -> Option<&Url> {
        let domain = url.domain()?;
        let mut included_because = None;
        let mut excluded_because = None;
        if self.include.is_empty() {
            included_because = Some(&WildCardedDomain::WildCard);
        }
        for wc_domain in &self.include {
            if wc_domain.matches(domain) {
                match included_because {
                    Some(prev) if !wc_domain.more_specific_than(prev) => (),
                    _ => included_because = Some(wc_domain),
                }
            }
        }
        for wc_domain in &self.exclude {
            if wc_domain.matches(domain) {
                match excluded_because {
                    Some(prev) if !wc_domain.more_specific_than(prev) => (),
                    _ => excluded_because = Some(wc_domain),
                }
            }
        }
        match (included_because, excluded_because) {
            (Some(include), Some(exclude)) if include.more_specific_than(exclude) => {
                Some(&self.url)
            }
            (Some(_), None) => Some(&self.url),
            _ => None,
        }
    }
}

/// A domain name, that optionally allows a * as its first subdomain.
#[derive(Clone, Debug)]
enum WildCardedDomain {
    WildCard,
    WildCarded(String),
    Exact(String),
}
impl WildCardedDomain {
    fn matches(&self, domain: &str) -> bool {
        match self {
            Self::WildCard => true,
            Self::WildCarded(d) => domain.ends_with(d),
            Self::Exact(d) => domain == d,
        }
    }

    fn more_specific_than(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::WildCard, Self::WildCard) => false,
            (_, Self::WildCard) => true,
            (Self::Exact(a), Self::WildCarded(_)) => other.matches(a),
            (Self::WildCarded(a), Self::WildCarded(b)) => a != b && a.ends_with(b),
            _ => false,
        }
    }
}
impl std::str::FromStr for WildCardedDomain {
    type Err = std::convert::Infallible;

    #[allow(clippy::string_slice)]
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(if s.starts_with("*.") {
            Self::WildCarded(s[1..].to_owned())
        } else if s == "*" {
            Self::WildCarded(String::new())
        } else {
            Self::Exact(s.to_owned())
        })
    }
}
impl<'de> Deserialize<'de> for WildCardedDomain {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::de::Deserializer<'de>,
    {
        crate::json::deserialize_from_str(deserializer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn partial(include: &[&str], exclude: &[&str]) -> PartialProxyConfig {
        let parse = |list: &[&str]| {
            list.iter()
                .map(|domain| domain.parse().expect("infallible"))
                .collect()
        };

        PartialProxyConfig {
            url: "socks5h://localhost:9050".parse().expect("valid url"),
            include: parse(include),
            exclude: parse(exclude),
        }
    }

    fn matches(proxy: &PartialProxyConfig, url: &str) -> bool {
        proxy.for_url(&url.parse().expect("valid url")).is_some()
    }

    /// The worked example from this module's doc comment.
    #[test]
    fn more_specific_include_beats_exclude() {
        let proxy = partial(
            &["*.onion", "matrix.myspecial.onion"],
            &["*.myspecial.onion"],
        );

        assert!(matches(&proxy, "http://ordinary.onion"));
        assert!(matches(&proxy, "http://matrix.myspecial.onion"));
        assert!(!matches(&proxy, "http://hello.myspecial.onion"));
    }

    #[test]
    fn empty_include_is_treated_as_wildcard() {
        let proxy = partial(&[], &[]);

        assert!(matches(&proxy, "http://anything.example"));
    }

    #[test]
    fn ip_literals_have_no_domain_and_never_match() {
        let proxy = partial(&[], &[]);

        assert!(!matches(&proxy, "http://127.0.0.1:8008"));
    }

    /// A preview aimed at the proxy's own hostname would be resolved here,
    /// which is the lookup the proxy endpoint is exempt from.
    #[test]
    fn resolver_alias_recognizes_a_locally_resolved_endpoint() {
        let config = ProxyConfig::Global {
            url: "socks5://tor.example:9050".parse().expect("valid url"),
        };

        assert!(config.resolver_alias(&"http://tor.example/".parse().expect("valid url")));
        assert!(!config.resolver_alias(&"http://example.org/".parse().expect("valid url")));
    }

    /// `socks5h` resolves the destination at the proxy, so nothing is looked
    /// up here and there is no exemption to alias.
    #[test]
    fn resolver_alias_ignores_a_remotely_resolved_endpoint() {
        let config = ProxyConfig::Global {
            url: "socks5h://tor.example:9050".parse().expect("valid url"),
        };

        assert!(!config.resolver_alias(&"http://tor.example/".parse().expect("valid url")));
    }

    /// A per-domain proxy only carries the domains it matches, so a URL it
    /// does not carry is resolved here even when it names the endpoint.
    #[test]
    fn resolver_alias_covers_an_unmatched_by_domain_endpoint() {
        let mut proxy = partial(&["*.onion"], &[]);
        proxy.url = "socks5h://tor.example:9050".parse().expect("valid url");

        let config = ProxyConfig::ByDomain(vec![proxy]);

        assert!(config.resolver_alias(&"http://tor.example/".parse().expect("valid url")));
    }

    #[test]
    fn resolver_alias_is_case_insensitive_over_the_host() {
        let config = ProxyConfig::Global {
            url: "socks5://Tor.Example:9050".parse().expect("valid url"),
        };

        assert!(config.resolver_alias(&"http://TOR.EXAMPLE/".parse().expect("valid url")));
    }

    #[test]
    fn resolver_alias_never_fires_without_a_proxy() {
        let config = ProxyConfig::None;

        assert!(!config.resolver_alias(&"http://example.org/".parse().expect("valid url")));
    }

    #[test]
    fn none_is_the_default_and_yields_no_proxy() {
        let config = ProxyConfig::default();

        assert!(matches!(config, ProxyConfig::None));
        assert!(config.to_proxy().expect("built").is_none());
    }

    #[test]
    fn global_proxy_deserializes_from_toml() {
        #[derive(serde::Deserialize)]
        struct Wrapper {
            proxy: ProxyConfig,
        }

        let wrapper: Wrapper = toml::from_str(
            r#"
            [proxy.global]
            url = "socks5h://localhost:9050"
            "#,
        )
        .expect("deserialized");

        let ProxyConfig::Global { url } = &wrapper.proxy else {
            panic!("expected a global proxy, got {:?}", wrapper.proxy);
        };
        assert_eq!(url.as_str(), "socks5h://localhost:9050");
        assert!(wrapper.proxy.to_proxy().expect("built").is_some());
    }
}
