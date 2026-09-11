use super::*;

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum IpLookupStrategy {
    Ipv4Only,

    Ipv6Only,

    Ipv4AndIpv6,

    Ipv6ThenIpv4,

    #[default]
    Ipv4ThenIpv6,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(transparent)]
pub(super) struct ListeningAddr {
    #[serde(with = "either::serde_untagged")]
    pub(super) addrs: Either<IpAddr, Vec<IpAddr>>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(transparent)]
pub(super) struct ListeningPort {
    #[serde(with = "either::serde_untagged")]
    pub(super) ports: Either<u16, Vec<u16>>,
}
