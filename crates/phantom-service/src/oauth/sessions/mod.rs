//! The authorizations, from the redirect out to the tokens they end in.
//!
//! A session is one user's authorization at one provider. It is created before
//! the user is sent to the provider — holding the PKCE verifier and the nonces
//! the callback is checked against — and outlives the redirect, because what
//! it carries afterwards is the provider's tokens and the Matrix account the
//! identity was bound to.
//!
//! # The three columns
//!
//! `oauthid_session` is the session itself, under the session id. The other
//! two are indexes onto it: `oauthuniqid_oauthid` from the hash of the
//! provider's issuer and subject, which is how a returning user is recognised
//! as the same person, and `userid_oauthid` from a Matrix user to every
//! session bound to them, which is how the account's authorizations are listed
//! and revoked. All three move together, in one transaction, so an index never
//! points at a session that is not there.
//!
//! # Why the identity key is locked
//!
//! A write batch cannot claim a key conditionally: two logins by the same
//! person arriving together would each read no association, each mint a
//! session, and the second would overwrite the first — leaving the earlier
//! session unreachable from the identity but still bound to the account. The
//! read and the write are therefore held under [`MutexMap`], keyed by the
//! identity rather than globally, so only the logins that could collide wait
//! on each other.

pub mod association;

use std::{sync::Arc, time::SystemTime};

use futures::{FutureExt, Stream, StreamExt, TryFutureExt, TryStreamExt};
use phantom_core::{
    Err, Result, implement,
    stream::{IterStream, ReadyExt, TryIgnore},
    sync::{MutexMap, MutexMapGuard},
};
use phantom_database::{Cbor, Database, Deserialized, Ignore, Map, Txn, serialize_to_vec};
use ruma::{OwnedUserId, UserId};
use serde::{Deserialize, Serialize};
use url::Url;

use super::{Provider, Providers, UserInfo, unique_id as session_unique_id};

pub struct Sessions {
    association_pending: std::sync::Mutex<association::Pending>,

    /// Serializes the read-check-write of each identity. See the module docs.
    write_locks: MutexMap<String, ()>,

    providers: Arc<Providers>,
    db: Data,
}

struct Data {
    oauthid_session: Arc<Map>,
    oauthuniqid_oauthid: Arc<Map>,
    userid_oauthid: Arc<Map>,
    database: Arc<Database>,
}

/// One authorization at one provider, from the redirect through to the tokens.
///
/// Nearly everything is optional because a session is written before any of it
/// is known: the redirect stores the provider and the PKCE verifier, the
/// callback adds the tokens, and the account binding arrives last.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Session {
    /// The provider this authorization is at, by its `client_id`.
    pub idp_id: Option<String>,

    /// This session's own id, which is the key it is stored under.
    pub sess_id: Option<SessionId>,

    /// Token type: `bearer`, `mac`, and so on.
    pub token_type: Option<String>,

    /// The access token the provider granted.
    pub access_token: Option<String>,

    /// The OpenID Connect ID token the provider returned.
    pub id_token: Option<String>,

    /// Seconds the access token was granted for.
    pub expires_in: Option<u64>,

    /// When the access token expires.
    pub expires_at: Option<SystemTime>,

    /// The token the access token is refreshed with.
    pub refresh_token: Option<String>,

    /// Seconds the refresh token was granted for.
    pub refresh_token_expires_in: Option<u64>,

    /// When the refresh token expires.
    pub refresh_token_expires_at: Option<SystemTime>,

    /// The scope actually granted, where the provider reports one.
    pub scope: Option<String>,

    /// Where to send the user once the authorization is complete.
    pub redirect_url: Option<Url>,

    /// The PKCE preimage, whose hash was sent to the provider as the
    /// challenge.
    pub code_verifier: Option<String>,

    /// A random string held only in the grant cookie, so the callback can tell
    /// it is the same browser that started the flow.
    pub cookie_nonce: Option<String>,

    /// A random single-use string passed through the provider's redirect.
    pub query_nonce: Option<String>,

    /// When the authorization grant itself expires — the window the user has
    /// to finish at the provider, not the lifetime of any token.
    pub authorize_expires_at: Option<SystemTime>,

    /// The Matrix account this identity is bound to.
    pub user_id: Option<OwnedUserId>,

    /// The last userinfo the provider answered with.
    pub user_info: Option<UserInfo>,
}

/// A session's identifier.
pub type SessionId = String;

/// Characters in the PKCE `code_verifier`. RFC 7636 §4.1 allows 43 to 128.
pub const CODE_VERIFIER_LENGTH: usize = 64;

/// Characters in a session id.
pub const SESSION_ID_LENGTH: usize = 32;

impl Sessions {
    pub(super) fn build(args: &crate::Args<'_>, providers: Arc<Providers>) -> Self {
        Self {
            association_pending: std::sync::Mutex::default(),
            write_locks: MutexMap::new(),
            providers,
            db: Data {
                oauthid_session: args.db["oauthid_session"].clone(),
                oauthuniqid_oauthid: args.db["oauthuniqid_oauthid"].clone(),
                userid_oauthid: args.db["userid_oauthid"].clone(),
                database: args.db.clone(),
            },
        }
    }
}

/// Deletes a session and every index that still points at it.
///
/// The identity index is only removed where it still names *this* session: a
/// newer authorization by the same person will have claimed it, and that
/// association has to survive the older session being cleaned up. The whole
/// removal is one transaction.
#[implement(Sessions)]
#[tracing::instrument(level = "debug", skip(self))]
pub async fn delete(&self, sess_id: &str) -> Result {
    let Some((session, unique_id, _write_guard)) = self.lock_for_delete(sess_id).await else {
        return Ok(());
    };

    // Hold on to an identity association that a newer session has taken over.
    let unique_id = async {
        let unique_id = unique_id.as_deref()?;
        let assoc_id = self
            .get_sess_id_by_unique_id(unique_id)
            .map(Result::ok)
            .await?;

        (assoc_id == sess_id).then_some(unique_id)
    }
    .await;

    let user_sessions = async {
        let user_id = session.user_id.as_deref()?;
        let sess_ids: Vec<_> = self
            .get_sess_id_by_user(user_id)
            .ready_filter_map(Result::ok)
            .ready_filter(|assoc_id| assoc_id != sess_id)
            .collect()
            .await;

        Some((user_id, sess_ids))
    }
    .await;

    let mut txn = Txn::new(&self.db.database.engine);

    if let Some((user_id, sess_ids)) = user_sessions {
        if sess_ids.is_empty() {
            txn.remove(&self.db.userid_oauthid, user_id.as_str());
        } else {
            txn.insert(
                &self.db.userid_oauthid,
                user_id.as_str(),
                serialize_to_vec(Cbor(&sess_ids))?,
            );
        }
    }

    if let Some(unique_id) = unique_id {
        txn.remove(&self.db.oauthuniqid_oauthid, unique_id);
    }

    txn.remove(&self.db.oauthid_session, sess_id);

    txn.execute()
}

/// Reads the session and takes the lock on its identity.
///
/// Both have to be true at once: the identity key is derived from the session,
/// so the session is read to find the key, and the key has to be held before
/// the session is trusted. The loop closes that gap — after taking the lock it
/// reads again, and starts over if the identity moved underneath it. `None`
/// where the session is already gone.
#[implement(Sessions)]
async fn lock_for_delete(
    &self,
    sess_id: &str,
) -> Option<(Session, Option<String>, Option<MutexMapGuard<String, ()>>)> {
    loop {
        let snapshot = self.get(sess_id).await.ok()?;
        let provider = self.provider(&snapshot).map(Result::ok).await;

        let unique_id = provider
            .as_ref()
            .and_then(|provider| session_unique_id((provider, &snapshot)).ok());

        let write_guard = match unique_id.as_deref() {
            Some(unique_id) => Some(self.write_locks.lock(unique_id).await),
            None => None,
        };

        let session = self.get(sess_id).await.ok()?;

        if session.idp_id.as_deref() != snapshot.idp_id.as_deref() {
            continue;
        }

        let current = provider
            .as_ref()
            .and_then(|provider| session_unique_id((provider, &session)).ok());

        if current == unique_id {
            return Some((session, unique_id, write_guard));
        }
    }
}

/// Writes a session and its indexes, in one transaction.
#[implement(Sessions)]
#[tracing::instrument(level = "debug", skip(self))]
pub async fn put(&self, session: &Session) -> Result {
    let unique_id = async {
        let provider = self.provider(session).map(Result::ok).await?;

        session_unique_id((&provider, session)).ok()
    }
    .await;

    let _write_guard = match unique_id.as_deref() {
        Some(unique_id) => Some(self.write_locks.lock(unique_id).await),
        None => None,
    };

    self.put_locked(session, unique_id.as_deref()).await
}

/// Builds and commits a session while holding its identity key.
///
/// `build` is handed the account the identity was last bound to, if any, and
/// returns the session to write along with whatever the caller wants to carry
/// out. Everything from the lookup to the commit is inside the lock, so two
/// logins by the same person cannot each decide the identity is new.
///
/// What comes back is the committed session, the caller's value, and the id of
/// the session this one displaced — which the caller usually wants to revoke.
#[implement(Sessions)]
#[tracing::instrument(level = "debug", skip_all)]
pub async fn commit_identity_session<T, F, Fut>(
    &self,
    unique_id: &str,
    build: F,
) -> Result<(Session, T, Option<SessionId>)>
where
    T: Send,
    F: FnOnce(Option<OwnedUserId>) -> Fut + Send,
    Fut: Future<Output = Result<(Session, T)>> + Send,
{
    let write_guard = self.write_locks.lock(unique_id).await;

    let existing = match self.get_by_unique_id(unique_id).await {
        Ok(session) => Some(session),
        Err(e) if e.is_not_found() => None,
        Err(e) => return Err(e),
    };

    let old_sess_id = existing
        .as_ref()
        .and_then(|session| session.sess_id.clone());

    let old_user_id = existing.and_then(|session| session.user_id);
    let (session, value) = build(old_user_id).await?;

    self.put_locked(&session, Some(unique_id)).await?;
    drop(write_guard);

    Ok((session, value, old_sess_id))
}

#[implement(Sessions)]
async fn put_locked(&self, session: &Session, unique_id: Option<&str>) -> Result {
    let Some(sess_id) = session.sess_id.as_deref() else {
        return Err!(Database("A session cannot be written without a sess_id"));
    };

    let user_sessions = async {
        let user_id = session.user_id.as_deref()?;
        let mut sess_ids: Vec<_> = self
            .get_sess_id_by_user(user_id)
            .ready_filter_map(Result::ok)
            .collect()
            .await;

        sess_ids.push(sess_id.to_owned());
        sess_ids.sort_unstable();
        sess_ids.dedup();

        Some((user_id, sess_ids))
    }
    .await;

    let mut txn = Txn::new(&self.db.database.engine);

    txn.insert(
        &self.db.oauthid_session,
        sess_id,
        serialize_to_vec(Cbor(session))?,
    );

    if let Some(unique_id) = unique_id {
        txn.insert(&self.db.oauthuniqid_oauthid, unique_id, sess_id);
    }

    if let Some((user_id, sess_ids)) = user_sessions {
        txn.insert(
            &self.db.userid_oauthid,
            user_id.as_str(),
            serialize_to_vec(Cbor(&sess_ids))?,
        );
    }

    txn.execute()
}

/// The session an identity is currently associated with.
#[implement(Sessions)]
#[tracing::instrument(level = "debug", skip(self), ret(level = "debug"))]
pub async fn get_by_unique_id(&self, unique_id: &str) -> Result<Session> {
    self.get_sess_id_by_unique_id(unique_id)
        .and_then(async |sess_id| self.get(&sess_id).await)
        .await
}

/// Every session bound to a Matrix account.
#[implement(Sessions)]
#[tracing::instrument(level = "debug", skip(self))]
pub fn get_by_user(&self, user_id: &UserId) -> impl Stream<Item = Result<Session>> + Send {
    self.get_sess_id_by_user(user_id)
        .and_then(async |sess_id| self.get(&sess_id).await)
}

/// The session of that id.
#[implement(Sessions)]
#[tracing::instrument(level = "debug", skip(self), ret(level = "debug"))]
pub async fn get(&self, sess_id: &str) -> Result<Session> {
    self.db
        .oauthid_session
        .get(sess_id)
        .await
        .deserialized::<Cbor<Session>>()
        .map(|Cbor(session)| session)
}

/// The ids of the sessions bound to a Matrix account.
#[implement(Sessions)]
#[tracing::instrument(level = "debug", skip(self))]
pub fn get_sess_id_by_user(
    &self,
    user_id: &UserId,
) -> impl Stream<Item = Result<SessionId>> + Send {
    self.db
        .userid_oauthid
        .get(user_id.as_str())
        .map(Deserialized::deserialized)
        .map_ok(|Cbor(sess_ids): Cbor<Vec<SessionId>>| sess_ids.into_iter())
        .map_ok(IterStream::try_stream)
        .try_flatten_stream()
}

/// The id of the session an identity is associated with.
#[implement(Sessions)]
#[tracing::instrument(level = "debug", skip(self), ret(level = "debug"))]
pub async fn get_sess_id_by_unique_id(&self, unique_id: &str) -> Result<SessionId> {
    self.db
        .oauthuniqid_oauthid
        .get(unique_id)
        .await
        .deserialized()
}

/// Every Matrix account with an authorization on it.
#[implement(Sessions)]
pub fn users(&self) -> impl Stream<Item = &UserId> + Send {
    self.db
        .userid_oauthid
        .keys::<&str>()
        .ignore_err()
        .map(|user_id| <&UserId>::try_from(user_id).expect("valid user id in db"))
}

/// Every session there is.
#[implement(Sessions)]
pub fn stream(&self) -> impl Stream<Item = Session> + Send {
    self.db
        .oauthid_session
        .stream()
        .ignore_err()
        .map(|(_, Cbor(session)): (Ignore, Cbor<Session>)| session)
}

/// The provider a session was authorized at, discovered.
#[implement(Sessions)]
pub async fn provider(&self, session: &Session) -> Result<Provider> {
    let Some(idp_id) = session.idp_id.as_deref() else {
        return Err!(Request(NotFound("This session names no provider")));
    };

    self.providers.get(idp_id).await
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use phantom_database::{Cbor, deserialize, serialize_to_vec};

    use super::{Session, SessionId};
    use crate::oauth::UserInfo;

    /// A session is stored as one CBOR value, so every field in it has to
    /// survive the round trip — including the ones the codec has no special
    /// case for. A field that silently does not is a session that cannot be
    /// read back, which is a login that fails at the last step.
    #[test]
    fn a_session_round_trips_through_the_codec() {
        let expires_at = UNIX_EPOCH + Duration::from_secs(1_757_000_000);

        let session = Session {
            idp_id: Some("client-id".to_owned()),
            sess_id: Some("session-id".to_owned()),
            access_token: Some("token".to_owned()),
            expires_in: Some(3600),
            expires_at: Some(expires_at),
            redirect_url: Some("https://example.com/callback?a=b".parse().expect("a URL")),
            code_verifier: Some("verifier".to_owned()),
            user_id: Some("@someone:example.com".try_into().expect("a user id")),
            user_info: Some(UserInfo {
                sub: "12345".to_owned(),
                email: Some("someone@example.com".to_owned()),
                ..Default::default()
            }),
            ..Default::default()
        };

        let bytes = serialize_to_vec(Cbor(&session)).expect("serializes");
        let Cbor(read): Cbor<Session> = deserialize(&bytes).expect("deserializes");

        assert_eq!(read.idp_id, session.idp_id);
        assert_eq!(read.sess_id, session.sess_id);
        assert_eq!(read.access_token, session.access_token);
        assert_eq!(read.expires_in, session.expires_in);
        assert_eq!(read.expires_at, Some(expires_at));
        assert_eq!(read.redirect_url, session.redirect_url);
        assert_eq!(read.user_id, session.user_id);
        assert_eq!(
            read.user_info.map(|info| info.sub).as_deref(),
            Some("12345")
        );
    }

    /// The default session is what a flow starts from, and it is written
    /// before all but one of its fields are known.
    #[test]
    fn an_empty_session_round_trips() {
        let bytes = serialize_to_vec(Cbor(&Session::default())).expect("serializes");
        let Cbor(read): Cbor<Session> = deserialize(&bytes).expect("deserializes");

        assert!(read.sess_id.is_none());
        assert!(read.user_info.is_none());
    }

    /// `userid_oauthid` holds the list of a user's sessions as one value, and
    /// the delete path rewrites it with an entry removed.
    #[test]
    fn a_session_id_list_round_trips() {
        let sess_ids: Vec<SessionId> = vec!["one".to_owned(), "two".to_owned()];

        let bytes = serialize_to_vec(Cbor(&sess_ids)).expect("serializes");
        let Cbor(read): Cbor<Vec<SessionId>> = deserialize(&bytes).expect("deserializes");

        assert_eq!(read, sess_ids);
    }
}
