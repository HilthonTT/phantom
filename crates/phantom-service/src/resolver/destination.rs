//! Where a federation request actually goes, once a server name has been
//! resolved.

use std::{
    borrow::Cow,
    fmt,
    net::{IpAddr, SocketAddr},
};

use arrayvec::ArrayString;
use phantom_core::math::Expected;
use serde::{Deserialize, Serialize};

/// The address half of a resolved destination: either an address to connect
/// to directly, or a name still to be resolved at connection time together
/// with the port to use.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub enum Destination {
    Literal(SocketAddr),
    Named(String, PortString),
}

/// A port, written with its leading colon so that it concatenates onto a host
/// directly. Numeric in practice, but a service name fits too.
pub type PortString = ArrayString<16>;

/// The port a Matrix server is assumed to listen on where nothing says
/// otherwise, with the leading colon [`PortString`] carries.
const DEFAULT_PORT: &str = ":8448";

/// The numeric form of [`DEFAULT_PORT`].
pub(crate) const DEFAULT_PORT_NUM: u16 = 8448;

/// Reads `dest` as an address literal, with the port it carries or the
/// default one. `None` where it is a name rather than an address.
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

/// Splits `dest` into a host and a port, supplying [`DEFAULT_PORT`] where it
/// has none.
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
    /// The destination as a URL to make a request against.
    pub fn https_string(&self) -> String {
        match self {
            Self::Literal(addr) => format!("https://{addr}"),
            Self::Named(host, port) => format!("https://{host}{port}"),
        }
    }

    /// The destination as it belongs in a `Host` header or a URI authority:
    /// the host and its port, without a scheme.
    pub fn uri_string(&self) -> String {
        match self {
            Self::Literal(addr) => addr.to_string(),
            Self::Named(host, port) => format!("{host}{port}"),
        }
    }

    /// The host alone, with no port.
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
            // The stored form keeps the leading colon; `strip_prefix` rather
            // than a slice so that a malformed value is `None` instead of a
            // panic on a non-boundary index.
            Self::Named(_, port) => port.strip_prefix(':')?.parse().ok(),
        }
    }

    #[inline]
    #[must_use]
    pub fn default_port() -> PortString {
        PortString::from(DEFAULT_PORT).expect("the default port fits a PortString")
    }

    /// Bytes this occupies, for the cache's memory report.
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
