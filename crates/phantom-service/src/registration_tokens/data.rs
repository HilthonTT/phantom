use std::{sync::Arc, time::SystemTime};

use phantom_database::Map;
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
    pub(super) fn new(expires: TokenExpires) -> Self {
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

impl std::fmt::Display for DatabaseTokenInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Token used {} times. {}", self.uses, self.expires)?;

        Ok(())
    }
}
