use super::prelude::*;

/// Serves the API over TLS directly, without a reverse proxy in front.
///
/// When `certs` and `key` are both set, every address in `address` and
/// `port` speaks HTTPS only, unless `ports` narrows TLS to some of them. Leave
/// both unset to serve plain HTTP behind a proxy that terminates TLS.
///
/// The files are read again on SIGUSR1, so a renewed certificate takes effect
/// without a restart. Changing the paths themselves needs a restart.
#[derive(Clone, Debug, Default, Deserialize)]
#[config_example_generator(filename = "phantom-example.toml", section = "global.tls")]
pub struct TlsConfig {
    /// Path to a PEM file with the certificate chain, leaf certificate first,
    /// such as Let's Encrypt's `fullchain.pem`.
    ///
    /// example: "/etc/letsencrypt/live/example.com/fullchain.pem"
    pub certs: Option<PathBuf>,

    /// Path to the PEM private key for `certs`, such as Let's Encrypt's
    /// `privkey.pem`. PKCS#1, PKCS#8 and SEC1 keys are accepted.
    ///
    /// example: "/etc/letsencrypt/live/example.com/privkey.pem"
    pub key: Option<PathBuf>,

    /// The listening ports that speak TLS; the other ports in `port` serve
    /// plain HTTP. Empty serves every port with TLS.
    ///
    /// This is for a client port behind a proxy (or on loopback) next to a
    /// federation port that peers reach directly, which is also the shape
    /// Matrix's Complement suite expects: plain HTTP on 8008, TLS on 8448.
    ///
    /// example: [8448]
    ///
    /// default: []
    #[serde(default)]
    pub ports: Vec<u16>,
}

impl TlsConfig {
    /// The certificate chain and key paths, when TLS is enabled.
    #[must_use]
    pub fn paths(&self) -> Option<(&PathBuf, &PathBuf)> {
        self.certs.as_ref().zip(self.key.as_ref())
    }

    /// Whether a listener on `port` speaks TLS, given that TLS is enabled.
    #[must_use]
    pub fn on_port(&self, port: u16) -> bool {
        self.ports.is_empty() || self.ports.contains(&port)
    }
}
