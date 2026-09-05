//! The token a client pages a space hierarchy with.
//!
//! A hierarchy is walked depth-first, and a page ends in the middle of that
//! walk. What has to survive to the next request is where the walk had got to
//! — which is a *path* from the root, not a room, because the same room can
//! appear at more than one place in a space tree and the walk has to resume at
//! the right one.
//!
//! The three parameters are carried along with it because the spec forbids
//! changing them mid-pagination. Carrying them in the token rather than
//! remembering them server-side means there is no per-client state to expire,
//! and a client that changes one is told so rather than being silently given a
//! differently-shaped tree.
//!
//! The encoding is deliberately opaque: base64url of compact JSON. A client
//! has no business reading it, and one that guesses at the format will be
//! broken by the next field added rather than by the first one it misreads.

use std::fmt::{Display, Formatter};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use phantom_core::{Result, err};
use serde::{Deserialize, Serialize};

use crate::rooms::short::ShortRoomId;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(super) struct PaginationToken {
    /// The path from the root of the hierarchy to the room the next page
    /// begins at, root first.
    #[serde(rename = "p")]
    pub(super) path: Vec<ShortRoomId>,

    /// The `limit` the pagination was started with.
    #[serde(rename = "l")]
    pub(super) limit: u64,

    /// The `max_depth` the pagination was started with.
    #[serde(rename = "d")]
    pub(super) max_depth: u64,

    /// The `suggested_only` the pagination was started with.
    #[serde(rename = "s")]
    pub(super) suggested_only: bool,
}

impl PaginationToken {
    /// Reads a token a client sent back.
    pub(super) fn decode(token: &str) -> Result<Self> {
        let json = URL_SAFE_NO_PAD
            .decode(token)
            .map_err(|e| err!(Request(InvalidParam("Invalid pagination token: {e}"))))?;

        serde_json::from_slice(&json)
            .map_err(|e| err!(Request(InvalidParam("Invalid pagination token: {e}"))))
    }

    /// Whether `other` was started with the same parameters as this one.
    ///
    /// The spec has the server reject a request that changes `limit`,
    /// `max_depth` or `suggested_only` while paginating, since each of them
    /// changes which rooms the walk visits and so what the path in the token
    /// means.
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
