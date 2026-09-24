//! Registration token validation and lifecycle management.
//!
//! The service combines static configuration tokens with database-backed tokens whose stored use
//! count or expiration time can invalidate them. Validation, consumption, listing, update, and
//! revocation share one interface.

mod data;

use std::{collections::HashSet, fmt::Display, sync::Arc};

use data::Data;
pub use data::{DatabaseTokenInfo, TokenExpires};
use futures::{Stream, StreamExt, pin_mut};
use phantom_core::{Err, Result, error, rand, stream::IterStream};

use crate::{Dep, ops::config};

const RANDOM_TOKEN_LENGTH: usize = 16;

/// Manages configured and database-backed registration tokens.
///
/// Configured tokens are read from the active configuration and optional token file on demand.
/// Database tokens retain use counts and expiration limits in persistent storage.
pub struct Service {
    db: Data,
    services: Services,
}

struct Services {
    config: Dep<config::Service>,
}

#[derive(Debug)]
pub struct ValidToken {
    /// Literal token accepted during registration.
    pub token: String,

    /// Origin and metadata associated with the token.
    pub info: TokenInfo,
}

impl Display for ValidToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "`{}` --- {}", self.token, self.info)
    }
}

impl PartialEq<str> for ValidToken {
    fn eq(&self, other: &str) -> bool {
        self.token == other
    }
}

#[derive(Clone, Copy, Debug)]
pub enum TokenInfo {
    /// Static token supplied by the homeserver.
    Config,

    /// Metadata loaded for a database-backed token.
    Database(DatabaseTokenInfo),
}

impl Display for TokenInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Config => write!(f, "Token defined in config file"),
            Self::Database(info) => info.fmt(f),
        }
    }
}

impl crate::Service for Service {
    fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
        Ok(Arc::new(Self {
            db: Data::new(args.db),
            services: Services {
                config: args.depend::<config::Service>("ops::config"),
            },
        }))
    }

    fn name(&self) -> &str {
        crate::make_name(std::module_path!())
    }
}

impl Service {
    /// Creates a database-backed registration token.
    ///
    /// A supplied token is stored verbatim; otherwise a random token of `length` characters is
    /// generated, defaulting to [`RANDOM_TOKEN_LENGTH`]. Existing database tokens are rejected.
    pub async fn create_token(
        &self,
        token: Option<&str>,
        length: Option<usize>,
        expires: TokenExpires,
    ) -> Result<(String, DatabaseTokenInfo)> {
        let token = token
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| rand::string(length.unwrap_or(RANDOM_TOKEN_LENGTH)));

        let info = self.db.save_token(&token, expires).await?;

        Ok((token, info))
    }

    /// Returns a token's origin and stored metadata, without a validity check.
    pub async fn get_token_info(&self, token: &str) -> Result<TokenInfo> {
        if self.get_config_tokens().await.contains(token) {
            return Ok(TokenInfo::Config);
        }

        self.db.get_token_info(token).await.map(TokenInfo::Database)
    }

    /// Replaces a database token's expiration policy, preserving its use count.
    pub async fn update_token(
        &self,
        token: &str,
        expires: TokenExpires,
    ) -> Result<DatabaseTokenInfo> {
        if self.get_config_tokens().await.contains(token) {
            return Err!(Request(Forbidden(
                "The token set in the config file cannot be updated"
            )));
        }

        self.db.update_token(token, expires).await
    }

    /// Reports whether at least one valid registration token is available.
    ///
    /// Invalid database entries encountered before the first valid one are removed.
    pub async fn is_enabled(&self) -> bool {
        let stream = self.iterate_tokens().await;

        pin_mut!(stream);

        stream.next().await.is_some()
    }

    /// Loads every registration token supplied by configuration.
    ///
    /// Whitespace-delimited tokens are read from the optional token file and combined with the
    /// inline token. Failure to read the file is logged and leaves only the inline token.
    pub async fn get_config_tokens(&self) -> HashSet<String> {
        let mut tokens = HashSet::new();

        if let Some(file) = &self.services.config.auth.registration_token_file {
            match tokio::fs::read_to_string(file).await {
                Ok(text) => tokens.extend(text.split_ascii_whitespace().map(ToOwned::to_owned)),
                Err(e) => error!("Failed to read the registration token file: {e}"),
            }
        }

        if let Some(token) = &self.services.config.auth.registration_token {
            tokens.insert(token.to_owned());
        }

        tokens
    }

    /// Validates a registration token without consuming a use.
    pub async fn is_token_valid(&self, token: &str) -> Result {
        self.check(token, false).await
    }

    /// Validates a registration token and consumes one permitted use.
    ///
    /// Configuration tokens are accepted without mutation. A database token is removed once its
    /// count reaches the stored threshold; consumers of one token are serialized by the data
    /// layer.
    pub async fn try_consume(&self, token: &str) -> Result {
        self.check(token, true).await
    }

    async fn check(&self, token: &str, consume: bool) -> Result {
        if self.get_config_tokens().await.contains(token)
            || self.db.check_token(token, consume).await
        {
            return Ok(());
        }

        Err!(Request(Forbidden("Registration token not valid")))
    }

    /// Revokes a database-backed registration token.
    pub async fn revoke_token(&self, token: &str) -> Result {
        if self.get_config_tokens().await.contains(token) {
            return Err!(Request(Forbidden(
                "The token set in the config file cannot be revoked. Edit the config file to \
                 change it."
            )));
        }

        self.db.revoke_token(token).await
    }

    /// Streams every currently valid registration token.
    ///
    /// Configuration tokens are yielded first, followed by valid database tokens. Invalid stored
    /// tokens are removed while the database stream is consumed.
    pub async fn iterate_tokens(&self) -> impl Stream<Item = ValidToken> + Send + '_ {
        let config_tokens = self
            .get_config_tokens()
            .await
            .into_iter()
            .map(|token| ValidToken {
                token,
                info: TokenInfo::Config,
            })
            .stream();

        let db_tokens = self
            .db
            .iterate_and_clean_tokens()
            .map(|(token, info)| ValidToken {
                token: token.to_owned(),
                info: TokenInfo::Database(info),
            });

        config_tokens.chain(db_tokens)
    }
}
