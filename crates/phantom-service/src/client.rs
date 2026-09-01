//! The HTTP clients every outbound request is made through.
//!
//! One client per kind of request rather than one for all of them. Each keeps
//! its own connection pool and its own timeouts, so a push gateway that has
//! stopped answering cannot hold connections a federation request needs, and
//! a URL preview cannot wait as long as a room join legitimately does.
//!
//! Requests are resolved through [`crate::resolver`] rather than through
//! reqwest's own DNS: the federation clients get the resolver's hooked
//! variant, which answers from the destination cache that
//! [`resolver::Service::resolve_destination`] fills in, so the address a
//! server name resolved to is the address the connection is made to.

use std::{sync::Arc, time::Duration};

use either::Either;
use ipaddress::IPAddress;
use phantom_core::{Config, Result, err, implement, info::user_agent, trace};
use reqwest::redirect;

use crate::resolver;

pub struct Service {
    /// Requests with no deadline of their own: URL previews and the
    /// announcement check.
    pub default: reqwest::Client,

    /// URL previews, which are the only requests aimed at an address a user
    /// chose. Bound to `url_preview_bound_interface` where one is configured.
    pub url_preview: reqwest::Client,

    /// Media fetched from another server on a user's behalf.
    pub extern_media: reqwest::Client,

    /// `.well-known/matrix/server` lookups. Short timeouts and no pooling:
    /// this runs while a federation request is already waiting on it, and the
    /// answer is cached for hours afterwards.
    pub well_known: reqwest::Client,

    /// Federation requests this server makes in the foreground.
    pub federation: reqwest::Client,

    /// Synapse's admin API, which some migration paths read from. Its
    /// endpoints stream for minutes, so the read timeout is its own.
    pub synapse: reqwest::Client,

    /// Transactions the sender pushes to other servers.
    pub sender: reqwest::Client,

    /// Requests to appservices, which usually sit on the same network.
    pub appservice: reqwest::Client,

    /// Requests to push gateways.
    pub pusher: reqwest::Client,

    /// `ip_range_denylist`, parsed once. See [`Self::valid_cidr_range`].
    pub cidr_range_denylist: Vec<IPAddress>,
}

impl crate::Service for Service {
    fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
        let config = &args.server.config;

        let resolver = args.require::<resolver::Service>("resolver");

        let url_preview_bind_addr = config
            .url_preview_bound_interface
            .clone()
            .and_then(Either::left);

        let url_preview_bind_iface = config
            .url_preview_bound_interface
            .clone()
            .and_then(Either::right);

        Ok(Arc::new(Self {
            default: base(config)?
                .dns_resolver(resolver.resolver.clone())
                .build()?,

            url_preview: builder_interface(base(config)?, url_preview_bind_iface.as_deref())?
                .local_address(url_preview_bind_addr)
                .dns_resolver(resolver.resolver.clone())
                .redirect(redirect::Policy::limited(3))
                .build()?,

            extern_media: base(config)?
                .dns_resolver(resolver.resolver.clone())
                .redirect(redirect::Policy::limited(3))
                .build()?,

            well_known: base(config)?
                .dns_resolver(resolver.resolver.clone())
                .connect_timeout(Duration::from_secs(config.well_known_conn_timeout))
                .read_timeout(Duration::from_secs(config.well_known_timeout))
                .timeout(Duration::from_secs(config.well_known_timeout))
                .pool_max_idle_per_host(0)
                .redirect(redirect::Policy::limited(4))
                .build()?,

            federation: base(config)?
                .dns_resolver(resolver.resolver.hooked.clone())
                .read_timeout(Duration::from_secs(config.federation_timeout))
                .pool_max_idle_per_host(config.federation_idle_per_host.into())
                .pool_idle_timeout(Duration::from_secs(config.federation_idle_timeout))
                .redirect(redirect::Policy::limited(3))
                .build()?,

            synapse: base(config)?
                .dns_resolver(resolver.resolver.hooked.clone())
                .read_timeout(Duration::from_secs(SYNAPSE_READ_TIMEOUT))
                .pool_max_idle_per_host(0)
                .redirect(redirect::Policy::limited(3))
                .build()?,

            sender: base(config)?
                .dns_resolver(resolver.resolver.hooked.clone())
                .read_timeout(Duration::from_secs(config.sender_timeout))
                .timeout(Duration::from_secs(config.sender_timeout))
                .pool_max_idle_per_host(1)
                .pool_idle_timeout(Duration::from_secs(config.sender_idle_timeout))
                .redirect(redirect::Policy::limited(2))
                .build()?,

            appservice: base(config)?
                .dns_resolver(resolver.resolver.clone())
                .connect_timeout(Duration::from_secs(APPSERVICE_CONN_TIMEOUT))
                .read_timeout(Duration::from_secs(config.appservice_timeout))
                .timeout(Duration::from_secs(config.appservice_timeout))
                .pool_max_idle_per_host(1)
                .pool_idle_timeout(Duration::from_secs(config.appservice_idle_timeout))
                .redirect(redirect::Policy::limited(2))
                .build()?,

            pusher: base(config)?
                .dns_resolver(resolver.resolver.clone())
                .pool_max_idle_per_host(1)
                .pool_idle_timeout(Duration::from_secs(config.pusher_idle_timeout))
                .redirect(redirect::Policy::limited(2))
                .build()?,

            cidr_range_denylist: config
                .ip_range_denylist
                .iter()
                .map(IPAddress::parse)
                .inspect(|cidr| trace!("Denied CIDR range: {cidr:?}"))
                .collect::<Result<_, String>>()
                .map_err(|e| err!(Config("ip_range_denylist", "{e}")))?,
        }))
    }

    fn name(&self) -> &str {
        crate::make_name(std::module_path!())
    }
}

/// Synapse's admin endpoints stream their results, and a large one can be
/// several minutes of trickle. Fixed rather than configured: nothing but a
/// migration reads them, and it is not a knob worth carrying in the config.
const SYNAPSE_READ_TIMEOUT: u64 = 305;

/// An appservice that is up answers a connection immediately — it is on the
/// same network in every deployment that makes sense. `appservice_timeout`
/// covers the work it does once connected.
const APPSERVICE_CONN_TIMEOUT: u64 = 5;

/// The settings every client starts from, before the ones that differ per
/// kind of request are applied over them.
fn base(config: &Config) -> Result<reqwest::ClientBuilder> {
    let builder = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(config.request_conn_timeout))
        .read_timeout(Duration::from_secs(config.request_timeout))
        .timeout(Duration::from_secs(config.request_total_timeout))
        .pool_idle_timeout(Duration::from_secs(config.request_idle_timeout))
        .pool_max_idle_per_host(config.request_idle_per_host.into())
        .user_agent(user_agent())
        .redirect(redirect::Policy::limited(6))
        .danger_accept_invalid_certs(config.allow_invalid_tls_certificates)
        .connection_verbose(cfg!(debug_assertions))
        .gzip(config.gzip_compression)
        .brotli(config.brotli_compression)
        .zstd(config.zstd_compression);

    match config.proxy.to_proxy()? {
        Some(proxy) => Ok(builder.proxy(proxy)),
        None => Ok(builder),
    }
}

/// Binds the builder to a network interface by name.
#[cfg(any(target_os = "android", target_os = "fuchsia", target_os = "linux"))]
fn builder_interface(
    builder: reqwest::ClientBuilder,
    iface: Option<&str>,
) -> Result<reqwest::ClientBuilder> {
    match iface {
        Some(iface) => Ok(builder.interface(iface)),
        None => Ok(builder),
    }
}

/// Rejects an interface name, which only the platforms above can bind to.
/// Reported here rather than ignored: the option exists to keep preview
/// traffic off a network, and silently not binding would do the opposite of
/// what it was set for.
#[cfg(not(any(target_os = "android", target_os = "fuchsia", target_os = "linux")))]
fn builder_interface(
    builder: reqwest::ClientBuilder,
    iface: Option<&str>,
) -> Result<reqwest::ClientBuilder> {
    use phantom_core::Err;

    match iface {
        Some(iface) => Err!(Config(
            "url_preview_bound_interface",
            "binding to the interface named {iface:?} is not supported on this platform; give an \
             address instead"
        )),
        None => Ok(builder),
    }
}

/// Whether an address may be connected to, per `ip_range_denylist`.
///
/// This is a backstop rather than a boundary — it is enforced in phantom, so
/// anything that reaches the network another way is not covered, and a proxy
/// is not accounted for at all. A firewall is what actually contains the
/// server.
#[inline]
#[must_use]
#[implement(Service)]
pub fn valid_cidr_range(&self, ip: &IPAddress) -> bool {
    self.cidr_range_denylist
        .iter()
        .all(|cidr| !cidr.includes(ip))
}
