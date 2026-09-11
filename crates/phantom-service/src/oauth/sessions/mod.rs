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

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Session {
    pub idp_id: Option<String>,

    pub sess_id: Option<SessionId>,

    pub token_type: Option<String>,

    pub access_token: Option<String>,

    pub id_token: Option<String>,

    pub expires_in: Option<u64>,

    pub expires_at: Option<SystemTime>,

    pub refresh_token: Option<String>,

    pub refresh_token_expires_in: Option<u64>,

    pub refresh_token_expires_at: Option<SystemTime>,

    pub scope: Option<String>,

    pub redirect_url: Option<Url>,

    pub code_verifier: Option<String>,

    pub cookie_nonce: Option<String>,

    pub query_nonce: Option<String>,

    pub authorize_expires_at: Option<SystemTime>,

    pub user_id: Option<OwnedUserId>,

    pub user_info: Option<UserInfo>,
}

pub type SessionId = String;

pub const CODE_VERIFIER_LENGTH: usize = 64;

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

#[implement(Sessions)]
#[tracing::instrument(level = "debug", skip(self))]
pub async fn delete(&self, sess_id: &str) -> Result {
    let Some((session, unique_id, _write_guard)) = self.lock_for_delete(sess_id).await else {
        return Ok(());
    };

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

#[implement(Sessions)]
#[tracing::instrument(level = "debug", skip(self), ret(level = "debug"))]
pub async fn get_by_unique_id(&self, unique_id: &str) -> Result<Session> {
    self.get_sess_id_by_unique_id(unique_id)
        .and_then(async |sess_id| self.get(&sess_id).await)
        .await
}

#[implement(Sessions)]
#[tracing::instrument(level = "debug", skip(self))]
pub fn get_by_user(&self, user_id: &UserId) -> impl Stream<Item = Result<Session>> + Send {
    self.get_sess_id_by_user(user_id)
        .and_then(async |sess_id| self.get(&sess_id).await)
}

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

#[implement(Sessions)]
#[tracing::instrument(level = "debug", skip(self), ret(level = "debug"))]
pub async fn get_sess_id_by_unique_id(&self, unique_id: &str) -> Result<SessionId> {
    self.db
        .oauthuniqid_oauthid
        .get(unique_id)
        .await
        .deserialized()
}

#[implement(Sessions)]
pub fn users(&self) -> impl Stream<Item = &UserId> + Send {
    self.db
        .userid_oauthid
        .keys::<&str>()
        .ignore_err()
        .map(|user_id| <&UserId>::try_from(user_id).expect("valid user id in db"))
}

#[implement(Sessions)]
pub fn stream(&self) -> impl Stream<Item = Session> + Send {
    self.db
        .oauthid_session
        .stream()
        .ignore_err()
        .map(|(_, Cbor(session)): (Ignore, Cbor<Session>)| session)
}

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

    #[test]
    fn an_empty_session_round_trips() {
        let bytes = serialize_to_vec(Cbor(&Session::default())).expect("serializes");
        let Cbor(read): Cbor<Session> = deserialize(&bytes).expect("deserializes");

        assert!(read.sess_id.is_none());
        assert!(read.user_info.is_none());
    }

    #[test]
    fn a_session_id_list_round_trips() {
        let sess_ids: Vec<SessionId> = vec!["one".to_owned(), "two".to_owned()];

        let bytes = serialize_to_vec(Cbor(&sess_ids)).expect("serializes");
        let Cbor(read): Cbor<Vec<SessionId>> = deserialize(&bytes).expect("deserializes");

        assert_eq!(read, sess_ids);
    }
}
