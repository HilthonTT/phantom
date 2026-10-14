//! Asking an identity server about a third-party identifier, for invites by
//! email address or phone number.
//!
//! The identity server is the one the inviting client names, with the access
//! token the client registered there, so every request here is made on a
//! user's behalf to a host the user chose. That is why each one is held to
//! the outbound denylist (`ip_range_denylist`): the identity client's resolver
//! leaves out denylisted addresses and it follows no redirect, so nothing is
//! sent to one. An IP literal is refused before connecting, and the address
//! connected to is checked again before the body is read.

use std::sync::Arc;

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use phantom_core::{Err, Result, err, hash::sha256, implement};
use reqwest::{Method, Url};
use ruma::{
    OwnedServerName, OwnedUserId, RoomId, UserId,
    third_party_invite::IdentityServerBase64PublicKey, thirdparty::Medium,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};

use crate::{Dep, net::client};

/// Ceiling on an identity server's answer. The largest is a lookup naming one
/// address; anything near this is not an identity server.
const RESPONSE_LIMIT: usize = 64 * 1024;

pub struct Service {
    services: Services,
}

struct Services {
    client: Dep<client::Service>,
}

impl crate::Service for Service {
    fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
        Ok(Arc::new(Self {
            services: Services {
                client: args.depend::<client::Service>(),
            },
        }))
    }

    fn name(&self) -> &str {
        crate::make_name(std::module_path!())
    }
}

/// `GET /_matrix/identity/v2/hash_details`: how the server wants lookups
/// hashed.
#[derive(Debug, Deserialize)]
pub struct HashDetails {
    pub algorithms: Vec<String>,
    pub lookup_pepper: String,
}

/// One address to look up, already in the form the server asked for.
#[derive(Debug, Eq, PartialEq)]
pub struct Lookup {
    pub algorithm: &'static str,
    pub pepper: String,
    pub address: String,
}

/// What a client sends to `POST /_matrix/identity/v2/store-invite`, beyond the
/// identifier itself: the room and the sender, so the identity server can
/// word the message it sends.
#[derive(Debug, Default, Serialize)]
pub struct InviteContext {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub room_alias: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub room_avatar_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub room_join_rules: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub room_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub room_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sender_display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sender_avatar_url: Option<String>,
}

/// The identity server's answer to `store-invite`: the token the eventual
/// `m.room.member` must carry signed, the keys that signature is checked
/// against, and a redacted form of the identifier to show in the room.
#[derive(Debug, Deserialize)]
pub struct StoredInvite {
    pub token: String,
    pub public_keys: Vec<StoredKey>,
    pub display_name: String,
}

#[derive(Debug, Deserialize)]
pub struct StoredKey {
    pub public_key: IdentityServerBase64PublicKey,
    pub key_validity_url: String,
}

#[derive(Debug, Deserialize)]
struct LookupResponse {
    #[serde(default)]
    mappings: std::collections::BTreeMap<String, String>,
}

/// The Matrix user an identifier is bound to on `id_server`, if any.
#[implement(Service)]
#[tracing::instrument(level = "debug", skip(self, access_token, address))]
pub async fn lookup(
    &self,
    id_server: &str,
    access_token: &str,
    medium: &Medium,
    address: &str,
) -> Result<Option<OwnedUserId>> {
    let details: HashDetails = self
        .request(
            id_server,
            access_token,
            Method::GET,
            "/_matrix/identity/v2/hash_details",
            None,
        )
        .await?;

    let lookup = lookup_request(&details, medium, address)?;
    let body = json!({
        "algorithm": lookup.algorithm,
        "pepper": lookup.pepper,
        "addresses": [lookup.address],
    });

    let response: LookupResponse = self
        .request(
            id_server,
            access_token,
            Method::POST,
            "/_matrix/identity/v2/lookup",
            Some(body),
        )
        .await?;

    Ok(response
        .mappings
        .get(&lookup.address)
        .and_then(|mxid| UserId::parse(mxid.as_str()).ok()))
}

/// Asks `id_server` to hold an invite for an identifier no one has bound yet,
/// and to tell its owner about it.
#[implement(Service)]
#[tracing::instrument(level = "debug", skip(self, access_token, address, context))]
#[expect(clippy::too_many_arguments)]
pub async fn store_invite(
    &self,
    id_server: &str,
    access_token: &str,
    medium: &Medium,
    address: &str,
    room_id: &RoomId,
    sender: &UserId,
    context: &InviteContext,
) -> Result<StoredInvite> {
    let mut body = serde_json::to_value(context)?;
    if let Value::Object(body) = &mut body {
        body.insert("medium".into(), medium.as_str().into());
        body.insert("address".into(), address.into());
        body.insert("room_id".into(), room_id.as_str().into());
        body.insert("sender".into(), sender.as_str().into());
    }

    let stored: StoredInvite = self
        .request(
            id_server,
            access_token,
            Method::POST,
            "/_matrix/identity/v2/store-invite",
            Some(body),
        )
        .await?;

    if stored.public_keys.is_empty() {
        return Err!(BadServerResponse(
            "Identity server {id_server} stored the invite but named no public key"
        ));
    }

    Ok(stored)
}

#[implement(Service)]
async fn request<T: DeserializeOwned>(
    &self,
    id_server: &str,
    access_token: &str,
    method: Method,
    path: &str,
    body: Option<Value>,
) -> Result<T> {
    let url = identity_url(id_server, path)?;
    let client = &self.services.client;

    if let Some(ip) = host_ip(&url)
        && !client.valid_cidr_range_ip(ip)
    {
        return Err!(Request(Forbidden(
            "The identity server {id_server} is a forbidden address."
        )));
    }

    // The identity client refuses denylisted addresses at connection time
    // and follows no redirect, so a name resolving inward, or a redirect
    // pointing there, is refused before anything is sent. The checks either
    // side of it remain for IP literals and for requests sent through a proxy.
    let mut request = client
        .identity
        .request(method, url.clone())
        .bearer_auth(access_token);

    if let Some(body) = body {
        request = request
            .header(http::header::CONTENT_TYPE, "application/json")
            .body(body.to_string());
    }

    let response = request.send().await.map_err(|e| {
        err!(Request(Unknown(
            "Could not reach the identity server {id_server}: {e}"
        )))
    })?;

    if let Some(remote_addr) = response.remote_addr()
        && !client.valid_cidr_range_remote_addr(response.url(), remote_addr)
    {
        return Err!(Request(Forbidden(
            "The identity server {id_server} is a forbidden address."
        )));
    }

    let status = response.status();
    let bytes = client::read_response_capped(response, RESPONSE_LIMIT).await?;

    if !status.is_success() {
        let detail = String::from_utf8_lossy(&bytes);

        return Err!(Request(Unknown(
            "The identity server {id_server} answered {status}: {detail}"
        )));
    }

    serde_json::from_slice(&bytes).map_err(|e| {
        err!(BadServerResponse(
            "Identity server {id_server} sent an unreadable answer to {path}: {e}"
        ))
    })
}

/// The URL of `path` on the identity server a client named.
///
/// A client gives the server as a bare host and optional port, which is the
/// shape of a Matrix server name; parsing it as one is what keeps a path, a
/// userinfo part or a second URL from riding along inside it.
pub fn identity_url(id_server: &str, path: &str) -> Result<Url> {
    let id_server = OwnedServerName::try_from(id_server).map_err(|e| {
        err!(Request(InvalidParam(
            "{id_server:?} is not an identity server: {e}"
        )))
    })?;

    Url::parse(&format!("https://{id_server}{path}"))
        .map_err(|e| err!(Request(InvalidParam("{id_server} gives no valid URL: {e}"))))
}

/// The form an address is looked up in, given what the server supports:
/// `sha256` when offered, as the spec prefers, else `none`.
///
/// An email address is lowercased first, since identity servers store them
/// that way; a phone number is already the bare digits a client sends.
pub fn lookup_request(details: &HashDetails, medium: &Medium, address: &str) -> Result<Lookup> {
    let address = match medium {
        Medium::Email => address.to_lowercase(),
        _ => address.to_owned(),
    };
    let medium = medium.as_str();
    let offered = |algorithm: &str| details.algorithms.iter().any(|a| a == algorithm);

    if offered("sha256") {
        let hashed = format!("{address} {medium} {}", details.lookup_pepper);

        return Ok(Lookup {
            algorithm: "sha256",
            pepper: details.lookup_pepper.clone(),
            address: URL_SAFE_NO_PAD.encode(sha256::hash(hashed.as_bytes())),
        });
    }

    if offered("none") {
        return Ok(Lookup {
            algorithm: "none",
            pepper: details.lookup_pepper.clone(),
            address: format!("{address} {medium}"),
        });
    }

    Err!(Request(Unknown(
        "The identity server offers no lookup algorithm this server supports."
    )))
}

fn host_ip(url: &Url) -> Option<std::net::IpAddr> {
    match url.host()? {
        url::Host::Ipv4(v4) => Some(v4.into()),
        url::Host::Ipv6(v6) => Some(v6.into()),
        url::Host::Domain(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use ruma::thirdparty::Medium;

    use super::{HashDetails, identity_url, lookup_request};

    fn details(algorithms: &[&str]) -> HashDetails {
        HashDetails {
            algorithms: algorithms.iter().map(|&a| a.to_owned()).collect(),
            lookup_pepper: "matrixrocks".to_owned(),
        }
    }

    #[test]
    fn sha256_is_preferred_and_hashed_as_the_spec_shows() {
        let lookup = lookup_request(
            &details(&["none", "sha256"]),
            &Medium::Email,
            "alice@example.com",
        )
        .unwrap();

        // The worked example from the identity service API's lookup section.
        assert_eq!(lookup.algorithm, "sha256");
        assert_eq!(lookup.pepper, "matrixrocks");
        assert_eq!(
            lookup.address,
            "4kenr7N9drpCJ4AfalmlGQVsOn3o2RHjkADUpXJWZUc"
        );
    }

    #[test]
    fn an_email_is_lowercased_before_hashing() {
        let upper =
            lookup_request(&details(&["sha256"]), &Medium::Email, "Alice@Example.COM").unwrap();
        let lower =
            lookup_request(&details(&["sha256"]), &Medium::Email, "alice@example.com").unwrap();

        assert_eq!(upper, lower);
    }

    #[test]
    fn none_sends_the_address_and_medium_in_the_clear() {
        let lookup = lookup_request(&details(&["none"]), &Medium::Msisdn, "447700900000").unwrap();

        assert_eq!(lookup.algorithm, "none");
        assert_eq!(lookup.address, "447700900000 msisdn");
    }

    #[test]
    fn an_unknown_algorithm_is_refused() {
        assert!(lookup_request(&details(&["md5"]), &Medium::Email, "a@b.c").is_err());
    }

    #[test]
    fn the_server_must_be_a_host_and_port() {
        assert_eq!(
            identity_url("vector.im", "/_matrix/identity/v2/lookup")
                .unwrap()
                .as_str(),
            "https://vector.im/_matrix/identity/v2/lookup"
        );
        assert_eq!(
            identity_url("id.example.com:8090", "/x").unwrap().as_str(),
            "https://id.example.com:8090/x"
        );

        for hostile in [
            "evil.com/path",
            "user@evil.com",
            "evil.com#",
            "https://evil.com",
            "",
        ] {
            assert!(
                identity_url(hostile, "/x").is_err(),
                "{hostile:?} was accepted"
            );
        }
    }
}
