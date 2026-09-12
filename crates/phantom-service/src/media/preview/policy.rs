use std::net::IpAddr;

use phantom_core::{Err, Result, debug, err, implement};
use reqwest::Url;
use url::Host;

use crate::media::Service;

#[implement(Service)]
pub(super) fn check_url_host(&self, url: &Url) -> Result {
    if self.services.client.proxy.resolver_alias(url) {
        return Err!(Request(Forbidden(
            "Requesting a locally resolved proxy endpoint is forbidden"
        )));
    }

    let host = url
        .host()
        .ok_or_else(|| err!(Request(Unknown("URL has no host"))))?;

    let ip = match host {
        Host::Domain(_) => return Ok(()),
        Host::Ipv4(v4) => IpAddr::V4(v4),
        Host::Ipv6(v6) => IpAddr::V6(v6),
    };

    if !self.services.client.valid_cidr_range_ip(ip) {
        return Err!(Request(Forbidden(
            "Requesting from this address is forbidden"
        )));
    }

    Ok(())
}

#[implement(Service)]
pub(super) fn check_remote_addr(&self, response: &reqwest::Response) -> Result {
    let Some(remote_addr) = response.remote_addr() else {
        return Err!(Request(Forbidden(
            "URL preview response has no peer address"
        )));
    };

    debug!(url = %response.url(), ?remote_addr, "URL preview response remote address");

    self.services
        .client
        .valid_cidr_range_remote_addr(response.url(), remote_addr)
        .then_some(())
        .ok_or_else(|| {
            err!(Request(Forbidden(
                "Requesting from this address is forbidden"
            )))
        })
}

#[implement(Service)]
pub fn url_preview_allowed(&self, url: &Url) -> bool {
    if ["http", "https"]
        .iter()
        .all(|&scheme| !scheme.eq_ignore_ascii_case(url.scheme()))
    {
        debug!("Ignoring non-HTTP/HTTPS URL to preview: {}", url);
        return false;
    }

    let host = match url.host_str() {
        None => {
            debug!(
                "Ignoring URL preview for a URL that does not have a host (?): {}",
                url
            );
            return false;
        }
        Some(h) => h.to_owned(),
    };

    let allowlist_domain_contains = &self
        .services
        .config
        .media
        .url_preview_domain_contains_allowlist;
    let allowlist_domain_explicit = &self
        .services
        .config
        .media
        .url_preview_domain_explicit_allowlist;
    let denylist_domain_explicit = &self
        .services
        .config
        .media
        .url_preview_domain_explicit_denylist;
    let allowlist_url_contains = &self
        .services
        .config
        .media
        .url_preview_url_contains_allowlist;

    if allowlist_domain_contains.contains(&"*".to_owned())
        || allowlist_domain_explicit.contains(&"*".to_owned())
        || allowlist_url_contains.contains(&"*".to_owned())
    {
        debug!(
            "Config key contains * which is allowing all URL previews. Allowing URL {}",
            url
        );
        return true;
    }

    if !host.is_empty() {
        if denylist_domain_explicit.contains(&host) {
            debug!(
                "Host {} is not allowed by url_preview_domain_explicit_denylist (check 1/4)",
                &host
            );
            return false;
        }

        if allowlist_domain_explicit.contains(&host) {
            debug!(
                "Host {} is allowed by url_preview_domain_explicit_allowlist (check 2/4)",
                &host
            );
            return true;
        }

        if allowlist_domain_contains
            .iter()
            .any(|domain_s| host.contains(domain_s))
        {
            debug!(
                "Host {} is allowed by url_preview_domain_contains_allowlist (check 3/4)",
                &host
            );
            return true;
        }

        if allowlist_url_contains
            .iter()
            .any(|url_s| url.as_str().contains(url_s))
        {
            debug!(
                "URL {} is allowed by url_preview_url_contains_allowlist (check 4/4)",
                url
            );
            return true;
        }

        if self.services.config.media.url_preview_check_root_domain {
            debug!("Checking root domain");

            match host.split_once('.') {
                None => return false,

                Some((_, root_domain)) => {
                    if denylist_domain_explicit.contains(&root_domain.to_owned()) {
                        debug!(
                            "Root domain {} is not allowed by \
                             url_preview_domain_explicit_denylist (check 1/3)",
                            root_domain
                        );
                        return false;
                    }

                    if allowlist_domain_explicit.contains(&root_domain.to_owned()) {
                        debug!(
                            "Root domain {} is allowed by url_preview_domain_explicit_allowlist \
                             (check 2/3)",
                            root_domain
                        );
                        return true;
                    }

                    if allowlist_domain_contains
                        .iter()
                        .any(|domain_s| root_domain.contains(domain_s.as_str()))
                    {
                        debug!(
                            "Root domain {} is allowed by url_preview_domain_contains_allowlist \
                             (check 3/3)",
                            root_domain
                        );
                        return true;
                    }
                }
            }
        }
    }

    false
}
