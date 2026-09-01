use super::*;

/// Which address records [`Config::ip_lookup_strategy`] asks for.
///
/// The reference spells this as a number 1 through 5, which nothing but its
/// own documentation can decode; the names the resolver already uses are
/// spelled out here instead.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum IpLookupStrategy {
    /// A records only.
    Ipv4Only,

    /// AAAA records only.
    Ipv6Only,

    /// Both at once; whichever answers first is used.
    Ipv4AndIpv6,

    /// AAAA, falling back to A.
    Ipv6ThenIpv4,

    /// A, falling back to AAAA.
    #[default]
    Ipv4ThenIpv6,
}

/// Accepts either a single address or a list of them.
#[derive(Clone, Debug, Deserialize)]
#[serde(transparent)]
pub(super) struct ListeningAddr {
    #[serde(with = "either::serde_untagged")]
    pub(super) addrs: Either<IpAddr, Vec<IpAddr>>,
}

/// Accepts either a single port or a list of them.
#[derive(Clone, Debug, Deserialize)]
#[serde(transparent)]
pub(super) struct ListeningPort {
    #[serde(with = "either::serde_untagged")]
    pub(super) ports: Either<u16, Vec<u16>>,
}
