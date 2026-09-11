use std::{
    net::{IpAddr, SocketAddr},
    sync::Arc,
    time::Duration,
};

use bytes::{Bytes, BytesMut};
use either::Either;
use ipaddress::IPAddress;
use phantom_core::{
    Config, Err, Result, config::proxy::ProxyConfig, debug, err, implement, info::user_agent, trace,
};
use reqwest::{Url, redirect};

use crate::resolver;

pub struct Service {
    pub default: reqwest::Client,

    pub url_preview: reqwest::Client,

    pub extern_media: reqwest::Client,

    pub well_known: reqwest::Client,

    pub federation: reqwest::Client,

    pub synapse: reqwest::Client,

    pub sender: reqwest::Client,

    pub appservice: reqwest::Client,

    pub pusher: reqwest::Client,

    pub oauth: reqwest::Client,

    pub cidr_range_denylist: Vec<IPAddress>,

    pub proxy: ProxyConfig,
}

impl crate::Service for Service {
    fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
        let config = &args.server.config;

        let resolver = args.require::<resolver::Service>("resolver");

        let url_preview_bind_addr = config
            .media
            .url_preview_bound_interface
            .clone()
            .and_then(Either::left);

        let url_preview_bind_iface = config
            .media
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
                .connect_timeout(Duration::from_secs(config.network.well_known_conn_timeout))
                .read_timeout(Duration::from_secs(config.network.well_known_timeout))
                .timeout(Duration::from_secs(config.network.well_known_timeout))
                .pool_max_idle_per_host(0)
                .redirect(redirect::Policy::limited(4))
                .build()?,

            federation: base(config)?
                .dns_resolver(resolver.resolver.hooked.clone())
                .read_timeout(Duration::from_secs(config.network.federation_timeout))
                .pool_max_idle_per_host(config.network.federation_idle_per_host.into())
                .pool_idle_timeout(Duration::from_secs(config.network.federation_idle_timeout))
                .redirect(redirect::Policy::limited(3))
                .build()?,

            oauth: base(config)?
                .dns_resolver(resolver.resolver.clone())
                .connect_timeout(Duration::from_secs(OAUTH_CONNECT_TIMEOUT))
                .read_timeout(Duration::from_secs(OAUTH_READ_TIMEOUT))
                .timeout(Duration::from_secs(OAUTH_READ_TIMEOUT))
                .redirect(redirect::Policy::none())
                .build()?,

            synapse: base(config)?
                .dns_resolver(resolver.resolver.hooked.clone())
                .read_timeout(Duration::from_secs(SYNAPSE_READ_TIMEOUT))
                .pool_max_idle_per_host(0)
                .redirect(redirect::Policy::limited(3))
                .build()?,

            sender: base(config)?
                .dns_resolver(resolver.resolver.hooked.clone())
                .read_timeout(Duration::from_secs(config.network.sender_timeout))
                .timeout(Duration::from_secs(config.network.sender_timeout))
                .pool_max_idle_per_host(1)
                .pool_idle_timeout(Duration::from_secs(config.network.sender_idle_timeout))
                .redirect(redirect::Policy::limited(2))
                .build()?,

            appservice: base(config)?
                .dns_resolver(resolver.resolver.clone())
                .connect_timeout(Duration::from_secs(APPSERVICE_CONN_TIMEOUT))
                .read_timeout(Duration::from_secs(config.network.appservice_timeout))
                .timeout(Duration::from_secs(config.network.appservice_timeout))
                .pool_max_idle_per_host(1)
                .pool_idle_timeout(Duration::from_secs(config.network.appservice_idle_timeout))
                .redirect(redirect::Policy::limited(2))
                .build()?,

            pusher: base(config)?
                .dns_resolver(resolver.resolver.clone())
                .pool_max_idle_per_host(1)
                .pool_idle_timeout(Duration::from_secs(config.network.pusher_idle_timeout))
                .redirect(redirect::Policy::limited(2))
                .build()?,

            cidr_range_denylist: config
                .network
                .ip_range_denylist
                .iter()
                .map(IPAddress::parse)
                .inspect(|cidr| trace!("Denied CIDR range: {cidr:?}"))
                .collect::<Result<_, String>>()
                .map_err(|e| err!(Config("ip_range_denylist", "{e}")))?,

            proxy: config.network.proxy.clone(),
        }))
    }

    fn name(&self) -> &str {
        crate::make_name(std::module_path!())
    }
}

const SYNAPSE_READ_TIMEOUT: u64 = 305;

const OAUTH_CONNECT_TIMEOUT: u64 = 10;
const OAUTH_READ_TIMEOUT: u64 = 30;

const APPSERVICE_CONN_TIMEOUT: u64 = 5;

fn base(config: &Config) -> Result<reqwest::ClientBuilder> {
    let builder = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(config.network.request_conn_timeout))
        .read_timeout(Duration::from_secs(config.network.request_timeout))
        .timeout(Duration::from_secs(config.network.request_total_timeout))
        .pool_idle_timeout(Duration::from_secs(config.network.request_idle_timeout))
        .pool_max_idle_per_host(config.network.request_idle_per_host.into())
        .user_agent(user_agent())
        .redirect(redirect::Policy::limited(6))
        .danger_accept_invalid_certs(config.network.allow_invalid_tls_certificates)
        .connection_verbose(cfg!(debug_assertions))
        .gzip(config.network.gzip_compression)
        .brotli(config.network.brotli_compression)
        .zstd(config.network.zstd_compression);

    match config.network.proxy.to_proxy()? {
        Some(proxy) => Ok(builder.proxy(proxy)),
        None => Ok(builder),
    }
}

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

#[inline]
#[must_use]
#[implement(Service)]
pub fn valid_cidr_range(&self, ip: &IPAddress) -> bool {
    self.cidr_range_denylist
        .iter()
        .all(|cidr| !cidr.includes(ip))
}

#[inline]
#[must_use]
#[implement(Service)]
pub fn valid_cidr_range_ip(&self, ip: IpAddr) -> bool {
    self.valid_cidr_range(&ipaddress_from_std(ip))
}

#[inline]
#[must_use]
#[implement(Service)]
pub fn valid_cidr_range_remote_addr(&self, url: &Url, remote_addr: SocketAddr) -> bool {
    self.valid_cidr_range_ip(remote_addr.ip()) || self.proxied(url)
}

#[inline]
#[must_use]
#[implement(Service)]
pub fn proxied(&self, url: &Url) -> bool {
    self.proxy.intercepts(url)
}

#[must_use]
fn ipaddress_from_std(ip: IpAddr) -> IPAddress {
    let (proto, text) = match ip {
        IpAddr::V4(v4) => ("ipv4", v4.to_string()),
        IpAddr::V6(v6) => ("ipv6", v6.to_string()),
    };

    IPAddress::parse(&text).unwrap_or_else(|e| {
        unreachable!("{proto} address {text} does not parse as one: {e}");
    })
}

pub async fn read_response_capped(mut response: reqwest::Response, limit: usize) -> Result<Bytes> {
    let mut body = match response.content_length() {
        Some(len) if len > limit.try_into().unwrap_or(u64::MAX) => {
            debug!(%len, %limit, "Rejecting response: the advertised body exceeds the limit");

            return Err!(BadServerResponse(
                "Response body length {len} exceeds the {limit} byte limit"
            ));
        }
        Some(len) => BytesMut::with_capacity(usize::try_from(len).unwrap_or(limit)),
        None => BytesMut::new(),
    };

    while let Some(chunk) = response.chunk().await? {
        if body.len().saturating_add(chunk.len()) > limit {
            debug!(%limit, "Rejecting response: the streamed body exceeds the limit");

            return Err!(BadServerResponse(
                "Response body exceeds the {limit} byte limit"
            ));
        }

        body.extend_from_slice(&chunk);
    }

    Ok(body.freeze())
}
