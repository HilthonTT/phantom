//! This server as an OpenID Connect provider.
//!
//! The other half of [`oauth`](super) points outwards, at the providers a user
//! authenticates against. This one points inwards: it is the provider Matrix
//! clients speaking next-gen auth (MSC3861 and the MSCs under it) talk to, so
//! that a client asks *this* server for a token rather than being handed one
//! by a login endpoint.
//!
//! It is optional, and it is built only when it can work — see [`can_build`].
//! Everything a client needs from a provider is here: registration
//! ([`client`]), the authorization code flow ([`auth`]), the device flow
//! ([`device`]), the signing key ([`signing_key`]) and the ID tokens
//! ([`token`]) and JWKS ([`jwk`]) that go with it.
//!
//! [`can_build`]: Server::can_build

mod auth;
mod client;
mod device;
mod jwk;
mod signing_key;
mod token;

use std::sync::Arc;

use phantom_core::{Err, Result, debug_info, debug_warn, err, implement, sync::MutexMap, warn};
use phantom_database::Map;
use ring::{rand::SystemRandom, signature::EcdsaKeyPair};
use serde_json::Value as JsonValue;

pub use self::{
    auth::{AUTH_REQUEST_LIFETIME, AuthCodeSession, AuthRequest},
    client::{ClientRegistration, DcrRequest},
    device::{
        ApprovedDeviceGrant, DEVICE_GRANT_INTERVAL_SECS, DEVICE_GRANT_LIFETIME, DeviceGrant,
        DeviceGrantPoll, DeviceGrantStatus, format_user_code,
    },
    token::IdTokenClaims,
};
use self::{
    jwk::init_jwk,
    signing_key::{SigningKey, init_signing_key, load_key_pair},
};
use crate::{Dep, config};

pub struct Server {
    services: Services,
    db: Data,

    /// The public half of [`key`](Self::key), as the JWKS serves it.
    jwk: JsonValue,

    key: SigningKey,

    /// [`key`](Self::key) parsed once, rather than per signature.
    key_pair: EcdsaKeyPair,

    rng: SystemRandom,

    /// Serializes the read-check-consume of one device grant, so two polls of
    /// an approved grant cannot both reach issuance. Keyed by `device_code`.
    device_locks: MutexMap<String, ()>,
}

struct Services {
    config: Dep<config::Service>,
}

struct Data {
    oidc_signingkey: Arc<Map>,
    oidcclientid_registration: Arc<Map>,
    oidccode_authsession: Arc<Map>,
    oidcdevicecode_devicegrant: Arc<Map>,
    oidcusercode_devicecode: Arc<Map>,
    oidcreqid_authrequest: Arc<Map>,
}

impl Server {
    /// Builds the OIDC server, or `None` where this deployment has no use for
    /// one.
    pub(super) fn build(args: &crate::Args<'_>) -> Result<Option<Self>> {
        if !Self::can_build(args) {
            return Ok(None);
        }

        let db = Data {
            oidc_signingkey: args.db["oidc_signingkey"].clone(),
            oidcclientid_registration: args.db["oidcclientid_registration"].clone(),
            oidccode_authsession: args.db["oidccode_authsession"].clone(),
            oidcdevicecode_devicegrant: args.db["oidcdevicecode_devicegrant"].clone(),
            oidcusercode_devicecode: args.db["oidcusercode_devicecode"].clone(),
            oidcreqid_authrequest: args.db["oidcreqid_authrequest"].clone(),
        };

        let rng = SystemRandom::new();
        let key = init_signing_key(&db)?;
        let key_pair = load_key_pair(&key.key_der, &rng)?;
        let jwk = init_jwk(&key_pair, &key.key_id)?;

        debug_info!(
            key = ?key.key_id,
            "Initializing the OIDC server for next-gen auth (MSC2965)"
        );

        Ok(Some(Self {
            services: Services {
                config: args.depend::<config::Service>("config"),
            },
            db,
            jwk,
            key,
            key_pair,
            rng,
            device_locks: MutexMap::new(),
        }))
    }

    /// Whether this deployment is configured for an OIDC server.
    ///
    /// It needs somewhere to be: the issuer and every endpoint under it are
    /// derived from `well_known_client`, and without that there is no URL to
    /// publish or to send a client back to. It also needs something to
    /// authenticate against — an identity provider, or `oidc_native_auth` for
    /// a server that authenticates users itself.
    fn can_build(args: &crate::Args<'_>) -> bool {
        let config = &args.server.config;
        let has_idp = !config.identity_provider.is_empty();
        let has_client_url = config.auth.well_known_client.is_some();
        let native = config.oidc.oidc_native_auth;

        if (has_idp || native) && !has_client_url {
            warn!("The OIDC server (next-gen auth) requires `well_known_client` to be set.");

            return false;
        }

        if !has_idp && !native {
            debug_warn!(
                "The OIDC server (next-gen auth) requires at least one `identity_provider`, or \
                 `oidc_native_auth` to be enabled."
            );

            return false;
        }

        true
    }
}

/// This server's issuer, which is what every endpoint is derived from.
///
/// Always ends in a slash: it is joined against, and a base without one would
/// have its last path component replaced rather than extended.
#[implement(Server)]
pub fn issuer_url(&self) -> Result<String> {
    self.services
        .config
        .auth
        .well_known_client
        .as_ref()
        .map(|url| {
            let url = url.to_string();

            if url.ends_with('/') {
                url
            } else {
                format!("{url}/")
            }
        })
        .ok_or_else(|| {
            err!(Config(
                "well_known_client",
                "well_known_client must be set for the OIDC server"
            ))
        })
}

/// The MSC2967 device scope, stable spelling first.
const DEVICE_SCOPE_PREFIXES: [&str; 2] = [
    "urn:matrix:client:device:",
    "urn:matrix:org.matrix.msc2967.client:device:",
];

/// The MSC2967 API scope, stable spelling first.
const API_SCOPE_PREFIXES: [&str; 2] = [
    "urn:matrix:client:api:",
    "urn:matrix:org.matrix.msc2967.client:api:",
];

/// Narrows a requested scope to what this server grants (RFC 6749 §3.3).
///
/// The tokens kept stay in the order they were asked for, since the granted
/// scope is echoed back to the client. An MSC2967 device scope is pulled out
/// separately, because the device id in it is what the token will be bound to.
/// Anything unrecognised is dropped, or refused outright under `strict`.
///
/// Asking for two devices at once, or for a device id outside the RFC 6749
/// scope-token character set, is an error rather than something to narrow: it
/// is a request this server cannot answer rather than one it can answer less
/// of.
pub fn narrow_scope(requested: &str, strict: bool) -> Result<(String, Option<String>)> {
    let mut granted = String::new();
    let mut device_id: Option<&str> = None;

    for token in requested.split_whitespace() {
        let keep = if let Some(id) = DEVICE_SCOPE_PREFIXES
            .iter()
            .find_map(|prefix| token.strip_prefix(prefix))
        {
            if device_id.is_some() {
                return Err!(Request(InvalidParam(
                    "More than one device scope requested"
                )));
            }
            if id.is_empty() || !id.bytes().all(is_scope_char) {
                return Err!(Request(InvalidParam(
                    "The device id contains an invalid character"
                )));
            }

            device_id = Some(id);
            true
        } else {
            token == "openid"
                || API_SCOPE_PREFIXES
                    .iter()
                    .any(|prefix| token.starts_with(prefix))
        };

        if keep {
            if !granted.is_empty() {
                granted.push(' ');
            }

            granted.push_str(token);
        } else if strict {
            return Err!(Request(InvalidParam("Unsupported scope requested")));
        }
    }

    Ok((granted, device_id.map(ToOwned::to_owned)))
}

/// RFC 6749 appendix A `NQCHAR`: printable ASCII other than space, `"` and
/// `\`. Wide enough for the unpadded base64 device ids MSC4108 clients use.
#[inline]
fn is_scope_char(b: u8) -> bool {
    b.is_ascii_graphic() && !matches!(b, b'"' | b'\\')
}

#[cfg(test)]
mod tests {
    use super::narrow_scope;

    #[test]
    fn narrow_scope_keeps_known_drops_unknown() {
        let requested =
            "openid urn:matrix:client:api:* urn:matrix:client:device:ABCDEFGHIJ custom:x";

        let (granted, device) = narrow_scope(requested, false).expect("narrows");

        assert_eq!(
            granted,
            "openid urn:matrix:client:api:* urn:matrix:client:device:ABCDEFGHIJ"
        );
        assert_eq!(device.as_deref(), Some("ABCDEFGHIJ"));
    }

    #[test]
    fn narrow_scope_strict_rejects_unknown() {
        narrow_scope("openid custom:x", true).unwrap_err();
        narrow_scope("openid custom:x", false).unwrap();
    }

    #[test]
    fn narrow_scope_accepts_unstable_device_spelling() {
        let scope = "urn:matrix:org.matrix.msc2967.client:device:DEV0123456";
        let (_granted, device) = narrow_scope(scope, false).expect("narrows");

        assert_eq!(device.as_deref(), Some("DEV0123456"));
    }

    #[test]
    fn narrow_scope_rejects_two_device_scopes() {
        let two = "urn:matrix:client:device:AAAAAAAAAA urn:matrix:client:device:BBBBBBBBBB";

        narrow_scope(two, false).unwrap_err();
    }

    #[test]
    fn narrow_scope_accepts_base64_device_id() {
        let scope = "urn:matrix:client:device:wjLpTLRqbqBzLs63aYaEv2Boi6cFEbbM/V+afGmU5+0";
        let (_granted, device) = narrow_scope(scope, false).expect("narrows");

        assert_eq!(
            device.as_deref(),
            Some("wjLpTLRqbqBzLs63aYaEv2Boi6cFEbbM/V+afGmU5+0")
        );
    }

    #[test]
    fn narrow_scope_rejects_invalid_device_id() {
        narrow_scope("urn:matrix:client:device:bad\"id", false).unwrap_err();
        narrow_scope("urn:matrix:client:device:bad\\id", false).unwrap_err();
    }
}
