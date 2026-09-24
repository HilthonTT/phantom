use std::{
    borrow::Cow,
    fmt,
    net::{IpAddr, SocketAddr},
};

use arrayvec::ArrayString;
use phantom_core::math::Expected;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub enum Destination {
    Literal(SocketAddr),
    Named(String, PortString),
}

pub type PortString = ArrayString<16>;

const DEFAULT_PORT: &str = ":8448";

pub(crate) const DEFAULT_PORT_NUM: u16 = 8448;

pub(crate) fn get_ip_with_port(dest_str: &str) -> Option<Destination> {
    if let Ok(dest) = dest_str.parse::<SocketAddr>() {
        Some(Destination::Literal(dest))
    } else if let Ok(ip_addr) = dest_str.parse::<IpAddr>() {
        Some(Destination::Literal(SocketAddr::new(
            ip_addr,
            DEFAULT_PORT_NUM,
        )))
    } else {
        None
    }
}

pub(crate) fn add_port_to_hostname(dest: &str) -> Destination {
    let (host, port) = match dest.find(':') {
        None => (dest, DEFAULT_PORT),
        Some(pos) => dest.split_at(pos),
    };

    Destination::Named(
        host.to_owned(),
        PortString::from(port).unwrap_or_else(|_| Destination::default_port()),
    )
}

impl Destination {
    pub fn https_string(&self) -> String {
        match self {
            Self::Literal(addr) => format!("https://{addr}"),
            Self::Named(host, port) => format!("https://{host}{port}"),
        }
    }

    pub fn uri_string(&self) -> String {
        match self {
            Self::Literal(addr) => addr.to_string(),
            Self::Named(host, port) => format!("{host}{port}"),
        }
    }

    #[inline]
    pub fn hostname(&self) -> Cow<'_, str> {
        match &self {
            Self::Literal(addr) => addr.ip().to_string().into(),
            Self::Named(host, _) => host.into(),
        }
    }

    #[inline]
    pub fn port(&self) -> Option<u16> {
        match &self {
            Self::Literal(addr) => Some(addr.port()),
            Self::Named(_, port) => port.strip_prefix(':')?.parse().ok(),
        }
    }

    #[inline]
    #[must_use]
    pub fn default_port() -> PortString {
        PortString::from(DEFAULT_PORT).expect("the default port fits a PortString")
    }

    #[inline]
    #[must_use]
    pub fn size(&self) -> usize {
        match self {
            Self::Literal(addr) => size_of_val(addr),
            Self::Named(host, port) => host.len().expected_add(port.capacity()),
        }
    }
}

impl fmt::Display for Destination {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.uri_string().as_str())
    }
}
