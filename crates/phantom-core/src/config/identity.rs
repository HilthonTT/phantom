use super::prelude::*;

#[derive(Clone, Debug, Deserialize)]
#[config_example_generator(
    filename = "phantom-example.toml",
    section = "global.identity_provider.example"
)]
pub struct IdentityProvider {
    pub brand: String,

    pub client_id: String,

    #[doc = "display: sensitive"]
    pub client_secret: Option<String>,

    pub client_secret_file: Option<PathBuf>,

    pub issuer_url: Option<Url>,

    pub callback_url: Option<Url>,

    #[serde(default)]
    pub default: bool,

    pub name: Option<String>,

    #[serde(default)]
    pub scope: BTreeSet<String>,

    pub base_path: Option<String>,

    #[serde(default = "true_fn")]
    pub discovery: bool,

    pub discovery_url: Option<Url>,

    pub authorization_url: Option<Url>,

    pub token_url: Option<Url>,

    pub revocation_url: Option<Url>,

    pub introspection_url: Option<Url>,

    pub userinfo_url: Option<Url>,
}

impl IdentityProvider {
    #[inline]
    #[must_use]
    pub fn id(&self) -> &str {
        self.client_id.as_str()
    }
}
