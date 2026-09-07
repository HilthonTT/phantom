//! The identity providers, as configured and as discovered.
//!
//! An operator configures the little that cannot be discovered — the brand,
//! the client credentials, the issuer — and discovery fills in the rest by
//! reading the provider's `openid-configuration`. What this holds is the
//! result of the two reconciled: the config with every endpoint resolved,
//! cached so that the reconciliation happens once rather than per login.
//!
//! The cache is keyed by the provider's `client_id`. A provider is also
//! reachable by its brand, but only where exactly one provider carries that
//! brand — with two GitHub applications configured, "github" names neither.

use std::collections::BTreeMap;

pub use phantom_core::config::IdentityProvider as Provider;
use phantom_core::{Err, Result, debug, err, implement};
use serde_json::{Map as JsonObject, Value as JsonValue};
use tokio::sync::RwLock;
use url::Url;

use crate::{Dep, client, client::read_response_capped, config};

/// The providers, as they stand after discovery.
pub struct Providers {
    services: Services,
    providers: RwLock<BTreeMap<ProviderId, Provider>>,
}

struct Services {
    client: Dep<client::Service>,
    config: Dep<config::Service>,
}

/// A provider's identifier, which is the `client_id` it was configured with.
pub type ProviderId = String;

impl Providers {
    pub(super) fn build(args: &crate::Args<'_>) -> Self {
        Self {
            services: Services {
                client: args.depend::<client::Service>("client"),
                config: args.depend::<config::Service>("config"),
            },
            providers: RwLock::new(BTreeMap::new()),
        }
    }
}

/// The provider of that id, discovered.
///
/// The first call for a provider reaches the network; every later one is
/// answered from the cache.
#[implement(Providers)]
#[tracing::instrument(level = "debug", skip(self))]
pub async fn get(&self, id: &str) -> Result<Provider> {
    if let Some(provider) = self.get_cached(id).await {
        return Ok(provider);
    }

    let config = self.get_config(id)?;
    let id = config.id().to_owned();
    let mut map = self.providers.write().await;
    let provider = self.configure(config).await?;

    debug!(?id, ?provider);
    map.insert(id, provider.clone());

    Ok(provider)
}

/// The provider of that id as the operator wrote it, before discovery.
///
/// The config is what is immutable here — it can be reloaded, but nothing in
/// this module writes to it — so this is the one lookup that never touches the
/// network. A provider is found by `client_id`, and failing that by brand,
/// which only identifies it while it is the only provider of that brand.
#[implement(Providers)]
pub fn get_config(&self, id: &str) -> Result<Provider> {
    let providers = &self.services.config.identity_provider;

    if let Some(provider) = providers.values().find(|config| config.id() == id) {
        return Ok(provider.clone());
    }

    if let Some(provider) = unique_by_brand(providers.values(), id) {
        return Ok(provider.clone());
    }

    Err!(Request(NotFound("Unrecognized identity provider")))
}

/// The id of the provider a bare `/login/sso/redirect` goes to.
///
/// The one an operator marked default, and the first one configured where none
/// is marked — with a single provider configured there is nothing else it
/// could mean.
#[implement(Providers)]
pub fn get_default_id(&self) -> Option<String> {
    let providers = &self.services.config.identity_provider;

    providers
        .values()
        .find(|provider| provider.default)
        .or_else(|| providers.values().next())
        .map(Provider::id)
        .map(ToOwned::to_owned)
}

/// The client secret of `provider`, from the config or the file it names.
///
/// Read on each exchange rather than held, so that rotating the file does not
/// need a restart.
#[implement(Providers)]
pub async fn get_client_secret(&self, provider: &Provider) -> Result<String> {
    if let Some(client_secret) = &provider.client_secret {
        return Ok(client_secret.clone());
    }

    let Some(path) = &provider.client_secret_file else {
        return Err!(Config(
            "identity_provider.client_secret",
            "Provider {:?} has neither a client_secret nor a client_secret_file",
            provider.client_id
        ));
    };

    let secret = tokio::fs::read_to_string(path).await.map_err(|e| {
        err!(Config(
            "identity_provider.client_secret_file",
            "Could not read the client secret of provider {:?} from {path:?}: {e}",
            provider.client_id
        ))
    })?;

    let secret = secret.trim();

    if secret.is_empty() {
        return Err!(Config(
            "identity_provider.client_secret_file",
            "The client secret file {path:?} of provider {:?} is empty",
            provider.client_id
        ));
    }

    Ok(secret.to_owned())
}

/// Drops the discovered configurations, so the next login rediscovers.
#[implement(Providers)]
pub async fn clear_cache(&self) {
    self.providers.write().await.clear();
}

/// The already-discovered provider of that id, by `client_id` or by a brand
/// that only one provider carries.
#[implement(Providers)]
async fn get_cached(&self, id: &str) -> Option<Provider> {
    let providers = self.providers.read().await;

    if let Some(provider) = providers.get(id) {
        return Some(provider.clone());
    }

    unique_by_brand(providers.values(), id).cloned()
}

/// Reconciles a configured provider with what it publishes.
///
/// Every URL the operator left unset is taken from the discovery document,
/// then from what the brand is known to use, then from the issuer by the
/// conventional path. What comes back is what the rest of this module treats
/// as the provider.
#[implement(Providers)]
#[tracing::instrument(level = "debug", ret(level = "debug"), skip(self))]
async fn configure(&self, mut provider: Provider) -> Result<Provider> {
    provider.name.get_or_insert_with(|| provider.brand.clone());

    if provider.issuer_url.is_none() {
        provider.issuer_url = Some(match provider.brand.as_str() {
            "github" => "https://github.com/login/oauth".try_into()?,
            "gitlab" => "https://gitlab.com".try_into()?,
            "google" => "https://accounts.google.com".try_into()?,
            _ => return Err!(Config("issuer_url", "Required for this provider.")),
        });
    }

    // MAS rejects `profile`, and its userinfo answers with the subject and a
    // username whether or not it is asked for anything else.
    if provider.scope.is_empty() && provider.brand == "mas" {
        provider.scope = ["openid".to_owned()].into();
    }

    let response = self
        .discover(&provider)
        .await
        .and_then(|response| {
            response.as_object().cloned().ok_or_else(|| {
                err!(Request(NotJson(
                    "Expecting a JSON object for the discovery response"
                )))
            })
        })
        .and_then(|response| check_issuer(response, &provider))?;

    if provider.authorization_url.is_none() {
        provider.authorization_url = discovered(&response, "authorization_endpoint")?
            .or_else(|| make_url(&provider, "authorize").ok());
    }

    if provider.revocation_url.is_none() {
        provider.revocation_url = discovered(&response, "revocation_endpoint")?
            .or_else(|| make_url(&provider, "revocation").ok());
    }

    if provider.introspection_url.is_none() {
        provider.introspection_url = discovered(&response, "introspection_endpoint")?
            .or_else(|| make_url(&provider, "introspection").ok());
    }

    if provider.userinfo_url.is_none() {
        provider.userinfo_url =
            discovered(&response, "userinfo_endpoint")?.or_else(|| match provider.brand.as_str() {
                "github" => "https://api.github.com/user".try_into().ok(),
                _ => make_url(&provider, "userinfo").ok(),
            });
    }

    if provider.token_url.is_none() {
        provider.token_url = discovered(&response, "token_endpoint")?.or_else(|| {
            // GitHub's is not `token`, and GitHub is not discoverable, so
            // nothing above will have supplied it.
            let path = if provider.brand == "github" {
                "access_token"
            } else {
                "token"
            };

            make_url(&provider, path).ok()
        });
    }

    if provider.callback_url.is_none()
        && let Some(server_url) = self.services.config.well_known_client.as_ref()
    {
        let path = format!(
            "_matrix/client/unstable/login/sso/callback/{}",
            provider.client_id
        );

        provider.callback_url = Some(server_url.join(&path)?);
    }

    Ok(provider)
}

/// Reads a provider's `openid-configuration`.
#[implement(Providers)]
#[tracing::instrument(level = "debug", ret(level = "trace"), skip(self))]
pub async fn discover(&self, provider: &Provider) -> Result<JsonValue> {
    let limit = self.services.config.oidc_max_response_size;
    let response = self
        .services
        .client
        .oauth
        .get(discovery_url(provider)?)
        .send()
        .await?
        .error_for_status()?;

    let body = read_response_capped(response, limit).await?;

    serde_json::from_slice(&body).map_err(Into::into)
}

/// The endpoint at `key` in a discovery document, parsed.
fn discovered(response: &JsonObject<String, JsonValue>, key: &str) -> Result<Option<Url>> {
    response
        .get(key)
        .and_then(JsonValue::as_str)
        .map(Url::parse)
        .transpose()
        .map_err(Into::into)
}

/// The provider of that brand, where it is the only one.
///
/// A brand shared by two providers names neither: answering with either of
/// them would be picking one of an operator's applications at random.
fn unique_by_brand<'a, I>(providers: I, brand: &str) -> Option<&'a Provider>
where
    I: Iterator<Item = &'a Provider> + Clone,
{
    let mut matching = providers.filter(|provider| provider.brand.eq_ignore_ascii_case(brand));
    let first = matching.next()?;

    matching.next().is_none().then_some(first)
}

/// Where a provider's `/.well-known/openid-configuration` is.
fn discovery_url(provider: &Provider) -> Result<Url> {
    if !provider.discovery {
        return Err!(Config(
            "discovery",
            "Discovery is disabled for provider {}",
            provider.id()
        ));
    }

    if let Some(url) = provider.discovery_url.clone() {
        return Ok(url);
    }

    make_url(provider, ".well-known/openid-configuration")
}

/// Checks that the issuer a provider claims is the one it was configured with.
///
/// The issuer is half of the key an identity is stored under, so a provider
/// answering under a different one would be handing this server's accounts to
/// a different set of people. Trailing slashes are not part of that: they are
/// not consistently written on either side.
//
// TODO: this is a string comparison. It says the response came from where the
// config said, not that it came from who it says — that needs the signed
// metadata of RFC 9207, which providers are still catching up with.
fn check_issuer(
    response: JsonObject<String, JsonValue>,
    provider: &Provider,
) -> Result<JsonObject<String, JsonValue>> {
    let expected = provider
        .issuer_url
        .as_ref()
        .map(Url::as_str)
        .map(|url| url.trim_end_matches('/'));

    let responded = response
        .get("issuer")
        .and_then(JsonValue::as_str)
        .map(|url| url.trim_end_matches('/'));

    if expected != responded {
        return Err!(Request(Unauthorized(
            "The configured issuer_url {expected:?} does not match the discovered {responded:?}",
        )));
    }

    Ok(response)
}

/// A URL at `path` under a provider's issuer, through any `base_path`.
///
/// An issuer whose path does not end in a slash has one added first: joining
/// against `https://host/realm` would otherwise replace `realm` rather than
/// descend into it.
fn make_url(provider: &Provider, path: &str) -> Result<Url> {
    let mut suffix = provider.base_path.clone().unwrap_or_default();
    suffix.push_str(path);

    let issuer = provider.issuer_url.as_ref().ok_or_else(|| {
        let id = &provider.client_id;
        err!(Config("issuer_url", "Provider {id:?} required field"))
    })?;

    let issuer_path = issuer.path();

    if issuer_path.ends_with('/') {
        return Ok(issuer.join(&suffix)?);
    }

    let mut url = issuer.clone();
    url.set_path(&format!("{issuer_path}/"));

    Ok(url.join(&suffix)?)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{Provider, make_url, unique_by_brand};

    fn provider(value: serde_json::Value) -> Provider {
        serde_json::from_value(value).expect("a provider configuration")
    }

    /// An issuer with a path is a realm, not a directory to be replaced.
    /// Joining onto it without the trailing slash would drop the realm and
    /// point every endpoint at the wrong place.
    #[test]
    fn an_issuer_path_is_descended_into_not_replaced() {
        let keycloak = provider(json!({
            "brand": "keycloak",
            "client_id": "id",
            "issuer_url": "https://sso.example.com/realms/matrix",
        }));

        assert_eq!(
            make_url(&keycloak, "token").expect("builds").as_str(),
            "https://sso.example.com/realms/matrix/token"
        );
    }

    #[test]
    fn an_issuer_already_ending_in_a_slash_is_not_given_another() {
        let idp = provider(json!({
            "brand": "generic",
            "client_id": "id",
            "issuer_url": "https://sso.example.com/",
        }));

        assert_eq!(
            make_url(&idp, ".well-known/openid-configuration")
                .expect("builds")
                .as_str(),
            "https://sso.example.com/.well-known/openid-configuration"
        );
    }

    /// `base_path` sits between the issuer and the path, for the providers
    /// that keep their `.well-known` somewhere other than the root of it.
    #[test]
    fn a_base_path_is_inserted_before_the_path() {
        let idp = provider(json!({
            "brand": "generic",
            "client_id": "id",
            "issuer_url": "https://sso.example.com/",
            "base_path": "auth/",
        }));

        assert_eq!(
            make_url(&idp, "token").expect("builds").as_str(),
            "https://sso.example.com/auth/token"
        );
    }

    #[test]
    fn a_brand_names_the_only_provider_carrying_it() {
        let providers = [
            provider(json!({ "brand": "github", "client_id": "gh" })),
            provider(json!({ "brand": "google", "client_id": "goog" })),
        ];

        let found = unique_by_brand(providers.iter(), "GitHub").expect("found by brand");

        assert_eq!(found.client_id, "gh");
    }

    /// With two applications of one brand configured, the brand names neither:
    /// answering with either would be picking one of them at random.
    #[test]
    fn a_shared_brand_names_no_provider() {
        let providers = [
            provider(json!({ "brand": "github", "client_id": "one" })),
            provider(json!({ "brand": "github", "client_id": "two" })),
        ];

        assert!(unique_by_brand(providers.iter(), "github").is_none());
    }
}
