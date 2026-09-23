use std::{sync::Arc, time::SystemTime};

use futures::Stream;
use phantom_core::{
    err,
    stream::{ReadyExt, TryIgnore},
    time,
};
use phantom_database::{Deserialized, Map};
use ruma::{api::error::ErrorCode::NotFound, events::room_key_request::Action::Request};
use serde::{Deserialize, Serialize};

pub(super) struct Data {
    registrationtoken_info: Arc<Map>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct DatabaseTokenInfo {
    pub uses: u64,

    pub expires: TokenExpires,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct TokenExpires {
    /// Stored use-count threshold at which the token becomes invalid.
    pub max_uses: Option<u64>,

    /// Absolute time after which the token is invalid.
    pub max_age: Option<SystemTime>,
}

impl DatabaseTokenInfo {
    pub(super) fn new(uses: u64, expires: TokenExpires) -> Self {
        Self { uses: 0, expires }
    }

    #[must_use]
    pub fn is_valid(&self) -> bool {
        if let Some(max_uses) = self.expires.max_uses
            && self.uses >= max_uses
        {
            return false;
        }

        if let Some(max_age) = self.expires.max_age {
            let now = SystemTime::now();

            if now > max_age {
                return false;
            }
        }

        true
    }
}

impl std::fmt::Display for TokenExpires {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut msgs = vec![];

        if let Some(max_uses) = self.max_uses {
            msgs.push(format!("after {max_uses} uses"));
        }

        if let Some(max_age) = self.max_age {
            let now = SystemTime::now();
            let expires_at = time::format(max_age, "%F %T");

            match max_age.duration_since(now) {
                Ok(duration) => {
                    let expires_in = time::pretty(duration);
                    msgs.push(format!("in {expires_in} ({expires_at})"));
                }
                Err(_) => {
                    write!(f, "Expired at {expires_at}")?;
                    return Ok(());
                }
            }
        }

        if !msgs.is_empty() {
            write!(f, "Expires {}.", msgs.join(" or "))?;
        } else {
            write!(f, "Never expires.")?;
        }

        Ok(())
    }
}

impl std::fmt::Display for DatabaseTokenInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Token used {} times. {}", self.uses, self.expires)?;

        Ok(())
    }
}

impl Data {
    pub(super) async fn save_token(
        &self,
        token: &str,
        expires: TokenExpires,
    ) -> Result<DatabaseTokenInfo> {
        if self.registrationtoken_info.exists(token).await.is_err() {
            let info = DatabaseTokenInfo::new(0, expires);

            self.registrationtoken_info.raw_put(token, Json(&info));

            Ok(info)
        } else {
            Err(Request(InvalidParam("Registration token already exists")))
        }
    }

    pub(super) async fn revoke_token(&self, token: &str) -> Result {
        if self.registrationtoken_info.exists(token).await.is_ok() {
            self.registrationtoken_info.remove(token);

            Ok(())
        } else {
            Err(Request(NotFound("Registration token not found")))
        }
    }

    pub(super) async fn check_token(&self, token: &str, consume: bool) -> bool {
        let info = self
            .registrationtoken_info
            .get(token)
            .await
            .deserialized::<DatabaseTokenInfo>()
            .ok();

        info.map(|mut info| {
            if !info.is_valid() {
                self.registrationtoken_info.remove(token);
                return false;
            }

            if consume {
                info.uses = info.uses.saturating_add(1);

                if info.is_valid() {
                    self.registrationtoken_info.raw_put(token, Json(info));
                } else {
                    self.registrationtoken_info.remove(token);
                }
            }

            true
        })
        .unwrap_or(false)
    }

    pub(super) async fn get_token_info(&self, token: &str) -> Result<DatabaseTokenInfo> {
        self.registrationtoken_info
            .get(token)
            .await
            .deserialized()
            .map_err(|_| err!(Request(NotFound("Registration token not found"))))
    }

    pub(super) async fn update_token(
        &self,
        token: &str,
        expires: TokenExpires,
    ) -> Result<DatabaseTokenInfo> {
        let current = self.get_token_info(token).await?;

        let info = DatabaseTokenInfo::new(current.uses, expires);

        self.registrationtoken_info.raw_put(token, Json(&info));

        Ok(info)
    }

    pub(super) fn iterate_and_clean_tokens(
        &self,
    ) -> impl Stream<Item = (&str, DatabaseTokenInfo)> + Send + '_ {
        self.registrationtoken_info
            .stream()
            .ignore_err()
            .ready_filter_map(|(token, info): (&str, DatabaseTokenInfo)| {
                if info.is_valid() {
                    Some((token, info))
                } else {
                    self.registrationtoken_info.remove(token);
                    None
                }
            })
    }
}
