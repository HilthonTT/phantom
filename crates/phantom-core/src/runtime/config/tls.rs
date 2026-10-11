use super::prelude::*;

/// Serves the API over TLS directly, without a reverse proxy in front.
///
/// When `certs` and `key` are both set, every address in `address` and
/// `port` speaks HTTPS only. Leave both unset to serve plain HTTP behind a
/// proxy that terminates TLS.
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
}

impl TlsConfig {
    /// The certificate chain and key paths, when TLS is enabled.
    #[must_use]
    pub fn paths(&self) -> Option<(&PathBuf, &PathBuf)> {
        self.certs.as_ref().zip(self.key.as_ref())
    }
}
