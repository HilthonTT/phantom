//! The name resolver itself, and the hook that lets reqwest read the
//! destination cache.
//!
//! [`Resolver`] is a plain hickory resolver configured from the system's own
//! settings and the `dns_*` config options. [`Hooked`] wraps it with a lookup
//! in [`super::cache`] first, and is what the federation clients resolve
//! through: by the time a federation request is made, the spec procedure in
//! [`super::lookup`] has already decided which address the connection belongs
//! to, and asking DNS again could answer differently.

use std::{net::SocketAddr, sync::Arc, time::Duration};

use futures::FutureExt;
use hickory_resolver::{
    TokioResolver,
    config::{ConnectionConfig, ProtocolConfig},
    lookup_ip::LookupIp,
    net::runtime::TokioRuntimeProvider,
};
use phantom_core::{Result, config::IpLookupStrategy, err, server::Server};
use reqwest::dns::{Addrs, Name, Resolve, Resolving};

use super::cache::{Cache, CachedOverride};

pub struct Resolver {
    pub(crate) resolver: Arc<TokioResolver>,
    pub(crate) hooked: Arc<Hooked>,
    server: Arc<Server>,
}

pub(crate) struct Hooked {
    resolver: Arc<TokioResolver>,
    cache: Arc<Cache>,
    server: Arc<Server>,
}

type ResolvingResult = Result<Addrs, Box<dyn std::error::Error + Send + Sync>>;

impl Resolver {
    pub(super) fn build(server: &Arc<Server>, cache: Arc<Cache>) -> Result<Arc<Self>> {
        let config = &server.config;

        let (sys_conf, mut opts) = hickory_resolver::system_conf::read_system_conf()
            .map_err(|e| err!("Failed to configure the DNS resolver from the system: {e}"))?;

        let mut conf = hickory_resolver::config::ResolverConfig::default();

        if let Some(domain) = sys_conf.domain() {
            conf.set_domain(domain.clone());
        }

        for search in sys_conf.search() {
            conf.add_search(search.clone());
        }

        for name_server in sys_conf.name_servers() {
            let mut ns = name_server.clone();

            if config.query_over_tcp_only {
                let port = ns.connections.first().map(|conn| conn.port);
                ns.connections
                    .retain(|conn| matches!(conn.protocol, ProtocolConfig::Tcp));

                if ns.connections.is_empty() {
                    let mut tcp = ConnectionConfig::tcp();
                    if let Some(port) = port {
                        tcp.port = port;
                    }
                    ns.connections.push(tcp);
                }
            }

            ns.trust_negative_responses = !config.query_all_nameservers;

            conf.add_name_server(ns);
        }

        opts.cache_size = u64::from(config.dns_cache_entries);
        opts.preserve_intermediates = true;
        opts.negative_min_ttl = Some(Duration::from_secs(config.dns_min_ttl_nxdomain));
        opts.negative_max_ttl = Some(Duration::from_secs(60 * 60 * 24 * 30));
        opts.positive_min_ttl = Some(Duration::from_secs(config.dns_min_ttl));
        opts.positive_max_ttl = Some(Duration::from_secs(60 * 60 * 24 * 7));
        opts.timeout = Duration::from_secs(config.dns_timeout);
        opts.attempts = config.dns_attempts.into();
        opts.try_tcp_on_error = config.dns_tcp_fallback;
        opts.num_concurrent_reqs = 1;
        opts.edns0 = true;
        opts.case_randomization = true;
        opts.ip_strategy = ip_strategy(config.ip_lookup_strategy);

        let mut builder = TokioResolver::builder_with_config(conf, TokioRuntimeProvider::new());
        *builder.options_mut() = opts;
        let resolver = Arc::new(
            builder
                .build()
                .map_err(|e| err!("Failed to build the DNS resolver: {e}"))?,
        );

        Ok(Arc::new(Self {
            resolver: resolver.clone(),
            hooked: Arc::new(Hooked {
                resolver,
                cache,
                server: server.clone(),
            }),
            server: server.clone(),
        }))
    }

    /// Empties hickory's own in-memory cache. The destination cache in the
    /// database is separate; see [`super::Service::clear_cache`].
    #[inline]
    pub fn clear_cache(&self) {
        self.resolver.clear_cache();
    }
}

fn ip_strategy(strategy: IpLookupStrategy) -> hickory_resolver::config::LookupIpStrategy {
    use hickory_resolver::config::LookupIpStrategy as Hickory;

    match strategy {
        IpLookupStrategy::Ipv4Only => Hickory::Ipv4Only,
        IpLookupStrategy::Ipv6Only => Hickory::Ipv6Only,
        IpLookupStrategy::Ipv4AndIpv6 => Hickory::Ipv4AndIpv6,
        IpLookupStrategy::Ipv6ThenIpv4 => Hickory::Ipv6thenIpv4,
        IpLookupStrategy::Ipv4ThenIpv6 => Hickory::Ipv4thenIpv6,
    }
}

impl Resolve for Resolver {
    fn resolve(&self, name: Name) -> Resolving {
        resolve_to_reqwest(self.server.clone(), self.resolver.clone(), name).boxed()
    }
}

impl Resolve for Hooked {
    fn resolve(&self, name: Name) -> Resolving {
        hooked_resolve(
            self.cache.clone(),
            self.server.clone(),
            self.resolver.clone(),
            name,
        )
        .boxed()
    }
}

#[tracing::instrument(level = "debug", skip_all, fields(name = ?name.as_str()))]
async fn hooked_resolve(
    cache: Arc<Cache>,
    server: Arc<Server>,
    resolver: Arc<TokioResolver>,
    name: Name,
) -> ResolvingResult {
    match cache.get_override(name.as_str()).await {
        Ok(cached) if cached.valid() => cached_to_reqwest(cached),

        Ok(CachedOverride {
            overriding: Some(overriding),
            ..
        }) => match overriding.parse() {
            Ok(name) => resolve_to_reqwest(server, resolver, name).boxed().await,
            Err(e) => Err(Box::new(e)),
        },

        _ => resolve_to_reqwest(server, resolver, name).boxed().await,
    }
}

async fn resolve_to_reqwest(
    server: Arc<Server>,
    resolver: Arc<TokioResolver>,
    name: Name,
) -> ResolvingResult {
    use std::{io, io::ErrorKind::Interrupted};

    let handle_shutdown = || -> Box<dyn std::error::Error + Send + Sync> {
        Box::new(io::Error::new(Interrupted, "Server shutting down"))
    };

    let handle_results = |results: LookupIp| -> Addrs {
        let addrs: Vec<_> = results.iter().map(|ip| SocketAddr::new(ip, 0)).collect();
        Box::new(addrs.into_iter())
    };

    tokio::select! {
        results = resolver.lookup_ip(name.as_str()) => Ok(handle_results(results?)),
        () = server.until_shutdown() => Err(handle_shutdown()),
    }
}

fn cached_to_reqwest(cached: CachedOverride) -> ResolvingResult {
    let addrs = cached
        .ips
        .into_iter()
        .map(move |ip| SocketAddr::new(ip, cached.port));

    Ok(Box::new(addrs))
}
