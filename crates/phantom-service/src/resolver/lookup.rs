//! The [spec procedure][spec] for resolving a server name, in the order the
//! spec gives it.
//!
//! The numbered steps in the comments below are the numbered bullets there.
//! Each is its own function, so a log line naming a step is enough to say
//! which branch a destination took.
//!
//! [spec]: https://spec.matrix.org/latest/server-server-api/#resolving-server-names

use std::{
    fmt::Debug,
    net::{IpAddr, SocketAddr},
};

use futures::{FutureExt, TryFutureExt};
use hickory_resolver::{net::NetError, proto::rr::RData};
use ipaddress::IPAddress;
use phantom_core::{Err, Result, debug, debug_info, err, error, result::LogErr, trace};
use ruma::ServerName;

use super::{
    cache::{CachedDest, CachedOverride, MAX_IPS},
    destination::{
        DEFAULT_PORT_NUM, Destination, PortString, add_port_to_hostname, get_ip_with_port,
    },
};

/// Where a federation request to a server goes, and what its `Host` header
/// should say.
#[derive(Clone, Debug)]
pub struct ResolvedDest {
    pub dest: Destination,
    pub host: String,
}

impl ResolvedDest {
    #[inline]
    pub fn string(&self) -> String {
        self.dest.https_string()
    }
}

impl super::Service {
    #[tracing::instrument(skip_all, level = "debug", name = "resolve")]
    pub async fn actual_destination(&self, server_name: &ServerName) -> Result<ResolvedDest> {
        let (CachedDest { dest, host, .. }, _cached) = self.lookup_destination(server_name).await?;

        Ok(ResolvedDest { dest, host })
    }

    /// The cached destination for `server_name`, resolving it if there is
    /// none. The flag says whether the answer came from the cache.
    pub async fn lookup_destination(&self, server_name: &ServerName) -> Result<(CachedDest, bool)> {
        if let Ok(result) = self.cache.get_destination(server_name).await {
            return Ok((result, true));
        }

        // Checked again under the lock: the resolution below is several round
        // trips, and everything that queued behind this guard would otherwise
        // repeat all of them after the first one had already cached an answer.
        let _dedup = self.resolving.lock(server_name.as_str()).await;
        if let Ok(result) = self.cache.get_destination(server_name).await {
            return Ok((result, true));
        }

        self.resolve_destination(server_name, true)
            .inspect_ok(|result| {
                self.cache
                    .set_destination(server_name, result)
                    .log_err()
                    .ok();
            })
            .map_ok(|result| (result, false))
            .boxed()
            .await
    }

    /// Runs the procedure itself. `cache` is what the admin command that
    /// resolves a name without disturbing the cache passes as false.
    #[tracing::instrument(name = "procedure", level = "debug", skip(self, cache))]
    pub async fn resolve_destination(&self, dest: &ServerName, cache: bool) -> Result<CachedDest> {
        self.validate_dest(dest)?;

        let mut host = dest.as_str().to_owned();
        let resolved = match get_ip_with_port(dest.as_str()) {
            Some(host_port) => Self::step_1(host_port)?,
            None => {
                if let Some(pos) = dest.as_str().find(':') {
                    self.step_2(dest, cache, pos).await?
                } else {
                    self.conditional_query_and_cache(dest.as_str(), DEFAULT_PORT_NUM, true)
                        .await?;

                    self.services.server.check_running()?;

                    match self.request_well_known(dest.as_str()).await? {
                        Some(delegated) => self.step_3(&mut host, cache, delegated).await?,
                        None => match self.query_srv_record(dest.as_str()).await? {
                            Some(overrider) => self.step_4(&host, cache, overrider).await?,
                            None => self.step_5(dest, cache).await?,
                        },
                    }
                }
            }
        };

        // Not `get_ip_with_port`: that supplies the default port, and the
        // `Host` header must carry a port only where the name itself did.
        let host = if let Ok(addr) = host.parse::<SocketAddr>() {
            Destination::Literal(addr)
        } else if let Ok(addr) = host.parse::<IpAddr>() {
            Destination::Named(addr.to_string(), Destination::default_port())
        } else if let Some(pos) = host.find(':') {
            let (host, port) = host.split_at(pos);
            Destination::Named(
                host.to_owned(),
                PortString::from(port).unwrap_or_else(|_| Destination::default_port()),
            )
        } else {
            Destination::Named(host, Destination::default_port())
        };

        debug!("Resolved to {resolved:?}, Host header {host:?}");

        Ok(CachedDest {
            dest: resolved,
            host: host.uri_string(),
            expire: CachedDest::default_expire(),
        })
    }

    fn step_1(host_port: Destination) -> Result<Destination> {
        debug!("1: IP literal with provided or default port");
        Ok(host_port)
    }

    async fn step_2(&self, dest: &ServerName, cache: bool, pos: usize) -> Result<Destination> {
        debug!("2: Hostname with included port");
        let (host, port) = dest.as_str().split_at(pos);

        self.conditional_query_and_cache(host, parse_port(port), cache)
            .await?;

        Ok(Destination::Named(
            host.to_owned(),
            PortString::from(port).unwrap_or_else(|_| Destination::default_port()),
        ))
    }

    async fn step_3(
        &self,
        host: &mut String,
        cache: bool,
        delegated: String,
    ) -> Result<Destination> {
        debug!("3: A .well-known file is available");
        *host = add_port_to_hostname(&delegated).uri_string();

        match get_ip_with_port(&delegated) {
            Some(host_and_port) => Self::step_3_1(host_and_port),
            None => {
                if let Some(pos) = delegated.find(':') {
                    self.step_3_2(cache, delegated, pos).await
                } else {
                    trace!("Delegated hostname has no port in this branch");
                    match self.query_srv_record(&delegated).await? {
                        Some(overrider) => self.step_3_3(cache, delegated, overrider).await,
                        None => self.step_3_4(cache, delegated).await,
                    }
                }
            }
        }
    }

    fn step_3_1(host_and_port: Destination) -> Result<Destination> {
        debug!("3.1: IP literal in .well-known file");
        Ok(host_and_port)
    }

    async fn step_3_2(&self, cache: bool, delegated: String, pos: usize) -> Result<Destination> {
        debug!("3.2: Hostname with port in .well-known file");
        let (host, port) = delegated.split_at(pos);

        self.conditional_query_and_cache(host, parse_port(port), cache)
            .await?;

        Ok(Destination::Named(
            host.to_owned(),
            PortString::from(port).unwrap_or_else(|_| Destination::default_port()),
        ))
    }

    async fn step_3_3(
        &self,
        cache: bool,
        delegated: String,
        overrider: Destination,
    ) -> Result<Destination> {
        debug!("3.3: SRV lookup successful");
        let force_port = overrider.port();

        self.conditional_query_and_cache_override(
            &delegated,
            &overrider.hostname(),
            force_port.unwrap_or(DEFAULT_PORT_NUM),
            cache,
        )
        .await?;

        if let Some(port) = force_port {
            return Ok(Destination::Named(delegated, port_string(port)));
        }

        Ok(add_port_to_hostname(&delegated))
    }

    async fn step_3_4(&self, cache: bool, delegated: String) -> Result<Destination> {
        debug!("3.4: No SRV records, just use the hostname from .well-known");
        self.conditional_query_and_cache(&delegated, DEFAULT_PORT_NUM, cache)
            .await?;

        Ok(add_port_to_hostname(&delegated))
    }

    async fn step_4(&self, host: &str, cache: bool, overrider: Destination) -> Result<Destination> {
        debug!("4: No .well-known; SRV record found");
        let force_port = overrider.port();

        self.conditional_query_and_cache_override(
            host,
            &overrider.hostname(),
            force_port.unwrap_or(DEFAULT_PORT_NUM),
            cache,
        )
        .await?;

        if let Some(port) = force_port {
            return Ok(Destination::Named(host.to_owned(), port_string(port)));
        }

        Ok(add_port_to_hostname(host))
    }

    async fn step_5(&self, dest: &ServerName, cache: bool) -> Result<Destination> {
        debug!("5: No SRV record found");
        self.conditional_query_and_cache(dest.as_str(), DEFAULT_PORT_NUM, cache)
            .await?;

        Ok(add_port_to_hostname(dest.as_str()))
    }

    #[inline]
    async fn conditional_query_and_cache(&self, hostname: &str, port: u16, cache: bool) -> Result {
        self.conditional_query_and_cache_override(hostname, hostname, port, cache)
            .await
    }

    #[inline]
    async fn conditional_query_and_cache_override(
        &self,
        untername: &str,
        hostname: &str,
        port: u16,
        cache: bool,
    ) -> Result {
        if !cache {
            return Ok(());
        }

        if self.cache.has_override(untername).await {
            return Ok(());
        }

        self.query_and_cache_override(untername, hostname, port)
            .await
    }

    /// Resolves `hostname` and stores the result under `untername`, which is
    /// the name a connection will later be opened to.
    #[tracing::instrument(name = "ip", level = "debug", skip(self))]
    async fn query_and_cache_override(
        &self,
        untername: &'_ str,
        hostname: &'_ str,
        port: u16,
    ) -> Result {
        self.services.server.check_running()?;

        debug!("querying IP for {untername:?} ({hostname:?}:{port})");

        match self.resolver.resolver.lookup_ip(hostname.to_owned()).await {
            Err(e) => Self::handle_resolve_error(&e, hostname),
            Ok(override_ip) => self.cache.set_override(
                untername,
                &CachedOverride {
                    ips: override_ip.iter().take(MAX_IPS).collect(),
                    port,
                    expire: CachedOverride::default_expire(),
                    overriding: (hostname != untername)
                        .then(|| hostname.to_owned())
                        .inspect(|_| debug_info!("{untername:?} overriden by {hostname:?}")),
                },
            ),
        }
    }

    /// The SRV records for `hostname`, under the current name and then the
    /// deprecated one.
    #[tracing::instrument(name = "srv", level = "debug", skip(self))]
    async fn query_srv_record(&self, hostname: &'_ str) -> Result<Option<Destination>> {
        let hostnames = [
            format!("_matrix-fed._tcp.{hostname}."),
            format!("_matrix._tcp.{hostname}."),
        ];

        for hostname in hostnames {
            self.services.server.check_running()?;

            debug!("querying SRV for {hostname:?}");
            let hostname = hostname.trim_end_matches('.');

            match self.resolver.resolver.srv_lookup(hostname).await {
                Err(e) => Self::handle_resolve_error(&e, hostname)?,
                Ok(result) => {
                    // `srv_lookup` answers with plain records now, so the SRV
                    // ones are picked out here.
                    let srv = result
                        .answers()
                        .iter()
                        .find_map(|record| match &record.data {
                            RData::SRV(srv) => Some(srv),
                            _ => None,
                        });

                    let Some(srv) = srv else {
                        // An empty answer is not the same as no record: fall
                        // through to the next name rather than reporting a
                        // destination with no target.
                        continue;
                    };

                    return Ok(Some(Destination::Named(
                        srv.target.to_string().trim_end_matches('.').to_owned(),
                        port_string(srv.port),
                    )));
                }
            }
        }

        Ok(None)
    }

    fn handle_resolve_error(e: &NetError, host: &'_ str) -> Result<()> {
        use hickory_resolver::net::DnsError;

        match e {
            // Not an error: a server that publishes no SRV record is the
            // ordinary case, and the caller moves on to the next step.
            NetError::Dns(DnsError::NoRecordsFound(..)) => {
                debug!(%host, "No DNS records found: {e}");
                Ok(())
            }
            NetError::Timeout => {
                Err!(warn!(host = %host, message = ::std::format_args!("DNS {e}")))
            }
            NetError::NoConnections => {
                error!(
                    "Your DNS server is overloaded and has run out of connections. Federation \
                     will be unreliable until this is remedied."
                );

                Err!(error!(host = %host, message = ::std::format_args!("DNS error: {e}")))
            }
            _ => Err!(error!(host = %host, message = ::std::format_args!("DNS error: {e}"))),
        }
    }

    /// Rejects a destination before anything is resolved for it.
    fn validate_dest(&self, dest: &ServerName) -> Result<()> {
        if dest == self.services.server.name && !self.services.server.config.federation_loopback {
            return Err!("Won't send federation request to ourselves");
        }

        if dest.is_ip_literal() || IPAddress::is_valid(dest.host()) {
            self.validate_dest_ip_literal(dest)?;
        }

        Ok(())
    }

    fn validate_dest_ip_literal(&self, dest: &ServerName) -> Result<()> {
        trace!("Destination is an IP literal, checking against the IP range denylist");
        debug_assert!(
            dest.is_ip_literal() || IPAddress::is_valid(dest.host()),
            "Destination is not an IP literal"
        );

        let ip = IPAddress::parse(dest.host()).map_err(|e| {
            err!(BadServerResponse(
                "Failed to parse IP literal from string: {e}"
            ))
        })?;

        self.validate_ip(&ip)?;

        Ok(())
    }

    pub fn validate_ip(&self, ip: &IPAddress) -> Result<()> {
        if !self.services.client.valid_cidr_range(ip) {
            return Err!(BadServerResponse("Not allowed to send requests to this IP"));
        }

        Ok(())
    }
}

/// A port as it is stored on a [`Destination::Named`], with the leading colon.
fn port_string(port: u16) -> PortString {
    PortString::from(format!(":{port}").as_str()).unwrap_or_else(|_| Destination::default_port())
}

/// The port out of a `":1234"` split off a hostname, falling back to the
/// default where it is not a number.
fn parse_port(port: &str) -> u16 {
    port.strip_prefix(':')
        .unwrap_or(port)
        .parse()
        .unwrap_or(DEFAULT_PORT_NUM)
}
