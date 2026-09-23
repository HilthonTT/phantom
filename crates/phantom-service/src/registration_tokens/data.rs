use std::{sync::Arc, time::SystemTime};

use futures::Stream;
use phantom_core::{
    Err, Result, err,
    stream::{ReadyExt, TryIgnore},
    sync::MutexMap,
    time,
};
use phantom_database::{Database, Deserialized, Json, Map};
use serde::{Deserialize, Serialize};

pub(super) struct Data {
    registrationtoken_info: Arc<Map>,

    /// Serializes the read-modify-write of one token, so two registrations
    /// racing on a single-use token cannot both read it unused.
    token_locks: MutexMap<String, ()>,
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
        Self { uses, expires }
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
    pub(super) fn new(db: &Arc<Database>) -> Self {
        Self {
            registrationtoken_info: db["registrationtoken_info"].clone(),
            token_locks: MutexMap::new(),
        }
    }

    pub(super) async fn save_token(
        &self,
        token: &str,
        expires: TokenExpires,
    ) -> Result<DatabaseTokenInfo> {
        let _lock = self.token_locks.lock(token).await;

        if self.registrationtoken_info.exists(token).await.is_err() {
            let info = DatabaseTokenInfo::new(0, expires);

            self.registrationtoken_info.raw_put(token, Json(&info))?;

            Ok(info)
        } else {
            Err!(Request(InvalidParam("Registration token already exists")))
        }
    }

    pub(super) async fn revoke_token(&self, token: &str) -> Result {
        let _lock = self.token_locks.lock(token).await;

        if self.registrationtoken_info.exists(token).await.is_ok() {
            self.registrationtoken_info.remove(token)
        } else {
            Err!(Request(NotFound("Registration token not found")))
        }
    }

    pub(super) async fn check_token(&self, token: &str, consume: bool) -> bool {
        let _lock = self.token_locks.lock(token).await;

        let info = self
            .registrationtoken_info
            .get(token)
            .await
            .deserialized::<DatabaseTokenInfo>()
            .ok();

        info.map(|mut info| {
            if !info.is_valid() {
                self.registrationtoken_info.remove(token).ok();
                return false;
            }

            if !consume {
                return true;
            }

            info.uses = info.uses.saturating_add(1);

            // A use that cannot be recorded is not granted, or a failing
            // write would let a limited token admit registrations forever.
            let recorded = if info.is_valid() {
                self.registrationtoken_info.raw_put(token, Json(info))
            } else {
                self.registrationtoken_info.remove(token)
            };

            recorded.is_ok()
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
        let _lock = self.token_locks.lock(token).await;

        let current = self.get_token_info(token).await?;

        let info = DatabaseTokenInfo::new(current.uses, expires);

        self.registrationtoken_info.raw_put(token, Json(&info))?;

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
                    self.registrationtoken_info.remove(token).ok();
                    None
                }
            })
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use super::{DatabaseTokenInfo, TokenExpires};

    const NEVER: TokenExpires = TokenExpires {
        max_uses: None,
        max_age: None,
    };

    #[test]
    fn new_keeps_the_use_count() {
        assert_eq!(DatabaseTokenInfo::new(7, NEVER).uses, 7);
    }

    #[test]
    fn unlimited_token_is_valid() {
        assert!(DatabaseTokenInfo::new(u64::MAX, NEVER).is_valid());
    }

    #[test]
    fn token_is_spent_at_max_uses() {
        let expires = TokenExpires {
            max_uses: Some(1),
            ..NEVER
        };

        assert!(DatabaseTokenInfo::new(0, expires).is_valid());
        assert!(!DatabaseTokenInfo::new(1, expires).is_valid());
    }

    #[test]
    fn token_lapses_at_max_age() {
        let now = SystemTime::now();
        let hour = Duration::from_secs(3600);

        let live = TokenExpires {
            max_age: Some(now + hour),
            ..NEVER
        };
        let lapsed = TokenExpires {
            max_age: Some(now - hour),
            ..NEVER
        };

        assert!(DatabaseTokenInfo::new(0, live).is_valid());
        assert!(!DatabaseTokenInfo::new(0, lapsed).is_valid());
    }
}
