use std::fmt::{Display, Formatter};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use phantom_core::{Result, err};
use serde::{Deserialize, Serialize};

use crate::rooms::short::ShortRoomId;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(super) struct PaginationToken {
    #[serde(rename = "p")]
    pub(super) path: Vec<ShortRoomId>,

    #[serde(rename = "l")]
    pub(super) limit: u64,

    #[serde(rename = "d")]
    pub(super) max_depth: u64,

    #[serde(rename = "s")]
    pub(super) suggested_only: bool,
}

impl PaginationToken {
    pub(super) fn decode(token: &str) -> Result<Self> {
        let json = URL_SAFE_NO_PAD
            .decode(token)
            .map_err(|e| err!(Request(InvalidParam("Invalid pagination token: {e}"))))?;

        serde_json::from_slice(&json)
            .map_err(|e| err!(Request(InvalidParam("Invalid pagination token: {e}"))))
    }

    pub(super) fn same_parameters(&self, other: &Self) -> bool {
        self.limit == other.limit
            && self.max_depth == other.max_depth
            && self.suggested_only == other.suggested_only
    }
}

impl Display for PaginationToken {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let json = serde_json::to_vec(self).expect("PaginationToken always serializes");

        f.write_str(&URL_SAFE_NO_PAD.encode(json))
    }
}

#[cfg(test)]
mod tests {
    use super::PaginationToken;

    #[test]
    fn round_trips() {
        let token = PaginationToken {
            path: vec![1, 2, 3],
            limit: 10,
            max_depth: 3,
            suggested_only: true,
        };

        assert_eq!(PaginationToken::decode(&token.to_string()).unwrap(), token);
    }

    #[test]
    fn rejects_garbage() {
        assert!(PaginationToken::decode("not a token").is_err());
        assert!(PaginationToken::decode("").is_err());
    }

    #[test]
    fn compares_parameters_not_position() {
        let a = PaginationToken {
            path: vec![1],
            limit: 10,
            max_depth: 3,
            suggested_only: false,
        };
        let b = PaginationToken {
            path: vec![1, 2],
            ..a.clone()
        };
        let c = PaginationToken {
            limit: 11,
            ..a.clone()
        };

        assert!(a.same_parameters(&b));
        assert!(!a.same_parameters(&c));
    }
}
