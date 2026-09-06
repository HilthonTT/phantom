use std::sync::Arc;

use phantom_core::{
    Result, at, debug, err,
    stream::{ReadyExt, TryIgnore},
};
use phantom_database::{Cbor, Database, Deserialized, Map};
#[cfg(feature = "url_preview")]
use phantom_database::{Txn, serialize_to_vec};
use ruma::{MxcUri, OwnedUserId, UserId};
#[cfg(feature = "url_preview")]
use serde::{Deserialize, Serialize};

use crate::media::preview::CachedPreview;

#[cfg(feature = "url_preview")]
use super::Media;

pub(crate) struct Data {
    pub mediaid_file: Arc<Map>,
    #[cfg(feature = "url_preview")]
    pub mediaid_lazy: Arc<Map>,
    #[cfg(feature = "url_preview")]
    pub mediaid_lazycontent: Arc<Map>,
    pub mediaid_pending: Arc<Map>,
    pub mediaid_user: Arc<Map>,
    pub url_previews: Arc<Map>,

    /// Held for [`Self::txn`], which is the only thing that needs the engine
    /// the columns live in rather than one column of it.
    #[cfg(feature = "url_preview")]
    pub db: Arc<Database>,
}

/// Borrowed staging-cache value: written zero-copy from the measured bytes.
#[cfg(feature = "url_preview")]
#[derive(Serialize)]
struct LazyContentRef<'a> {
    content_type: Option<&'a str>,
    content_disposition: Option<&'a str>,
    #[serde(with = "serde_bytes")]
    content: &'a [u8],
}

/// Owned staging-cache value read back at promotion. `ContentDisposition` is
/// Serialize-only, so the disposition rides as its header string.
#[cfg(feature = "url_preview")]
#[derive(Debug, Deserialize)]
struct LazyContent {
    content_type: Option<String>,
    content_disposition: Option<String>,
    #[serde(with = "serde_bytes")]
    content: Vec<u8>,
}

#[cfg(feature = "url_preview")]
impl From<LazyContent> for Media {
    fn from(lazy: LazyContent) -> Self {
        let content_disposition = lazy
            .content_disposition
            .and_then(|disposition| disposition.parse().ok());

        Self {
            content: lazy.content,
            content_type: lazy.content_type,
            content_disposition,
        }
    }
}

impl Data {
    pub(super) fn new(db: &Arc<Database>) -> Self {
        Self {
            #[cfg(feature = "url_preview")]
            db: db.clone(),
            mediaid_file: db["mediaid_file"].clone(),
            #[cfg(feature = "url_preview")]
            mediaid_lazy: db["mediaid_lazy"].clone(),
            #[cfg(feature = "url_preview")]
            mediaid_lazycontent: db["mediaid_lazycontent"].clone(),
            mediaid_pending: db["mediaid_pending"].clone(),
            mediaid_user: db["mediaid_user"].clone(),
            url_previews: db["url_previews"].clone(),
        }
    }

    /// Records a media id as reserved by a user until `expires_at`, which is
    /// milliseconds since the epoch.
    ///
    /// Keyed by the whole URI rather than by its parts: a reservation is only
    /// ever looked up by the exact id a client was handed, and it lives for a
    /// day at most.
    pub(super) fn insert_pending(&self, mxc: &MxcUri, user: &UserId, expires_at: u64) -> Result {
        self.mediaid_pending.raw_put(mxc, (expires_at, user))
    }

    /// Drops a reservation, which filling one finishes with.
    pub(super) fn remove_pending(&self, mxc: &MxcUri) -> Result {
        self.mediaid_pending.remove(mxc.as_str())
    }

    /// Who reserved a media id, and when their claim on it expires.
    pub(super) async fn search_pending(&self, mxc: &MxcUri) -> Result<(OwnedUserId, u64)> {
        self.mediaid_pending
            .get(mxc.as_str())
            .await
            .deserialized()
            .map(|(expires_at, user): (u64, OwnedUserId)| (user, expires_at))
    }

    /// How many unexpired reservations a user holds as of `now`, and when the
    /// first of them expires.
    ///
    /// Scans the column rather than indexing by user: reservations are few
    /// and short-lived by construction, since holding many of them is the
    /// thing this count is read to refuse.
    ///
    /// Expired rows are dropped on the way past. Nothing else ever visits
    /// them — a reservation is looked up only by the id it named — so without
    /// this the column would grow without bound, and a user who let five
    /// reservations expire could never make another.
    pub(super) async fn count_pending_for(&self, user: &UserId, now: u64) -> (usize, u64) {
        type KeyVal<'a> = (&'a str, (u64, &'a str));

        let held = (0_usize, u64::MAX, Vec::new());
        let (count, earliest, expired) = self
            .mediaid_pending
            .stream()
            .ignore_err()
            .ready_fold(
                held,
                |(count, earliest, mut expired), (mxc, (expires_at, holder)): KeyVal<'_>| {
                    if expires_at <= now {
                        expired.push(mxc.to_owned());
                    } else if holder == user.as_str() {
                        return (count.saturating_add(1), earliest.min(expires_at), expired);
                    }

                    (count, earliest, expired)
                },
            )
            .await;

        for mxc in expired {
            debug!(?mxc, "Dropping an expired media reservation");

            self.mediaid_pending.remove(mxc.as_str()).ok();
        }

        (count, earliest)
    }

    #[cfg(feature = "url_preview")]
    /// A transaction over this database's columns.
    pub(super) fn txn(&self) -> Txn {
        Txn::new(&self.db.engine)
    }

    pub(super) async fn get_url_preview(&self, url: &str) -> Result<CachedPreview> {
        self.url_previews
            .get(url)
            .await
            .deserialized::<Cbor<_>>()
            .map(at!(0))
            .ok()
            .filter(CachedPreview::is_valid)
            .ok_or(err!(Request(NotFound("Expired from cache"))))
    }

    /// Caches a preview under the URL it was generated for.
    pub(super) fn set_url_preview(&self, url: &str, cached: &CachedPreview) -> Result {
        self.url_previews.raw_put(url, Cbor(cached))
    }

    #[cfg(feature = "url_preview")]
    /// Records the external URL a lazy media mxc stands for, so that a client
    /// asking for it is what fetches it (see `Service::fetch_lazy_media`).
    pub(super) fn insert_lazy_media(&self, mxc: &str, url: &str) -> Result {
        debug!(?mxc, ?url, "Registering lazy media");

        self.mediaid_lazy.insert(mxc, url.as_bytes())
    }

    #[cfg(feature = "url_preview")]
    /// [`Self::insert_lazy_media`] as part of a transaction, for when the
    /// registration lands together with the bytes it stages.
    pub(super) fn queue_lazy_media(&self, txn: &mut Txn, mxc: &str, url: &str) {
        debug!(?mxc, ?url, "Registering lazy media");

        txn.insert(&self.mediaid_lazy, mxc, url.as_bytes());
    }

    #[cfg(feature = "url_preview")]
    /// Drops a lazy media registration, which is what promoting it into the
    /// media store finishes with.
    pub(super) fn remove_lazy_media(&self, txn: &mut Txn, mxc: &str) {
        txn.remove(&self.mediaid_lazy, mxc);
    }

    #[cfg(feature = "url_preview")]
    /// The external URL a lazy media mxc stands for, where it is still one.
    pub(super) async fn search_lazy_media(&self, mxc: &str) -> Result<String> {
        self.mediaid_lazy.get(mxc).await.and_then(|handle| {
            std::str::from_utf8(&handle)
                .map(ToOwned::to_owned)
                .map_err(|e| err!(Database("Lazy media {mxc} has an invalid URL: {e}")))
        })
    }

    #[cfg(feature = "url_preview")]
    /// Stages the bytes a preview already fetched, so the first client
    /// download promotes them rather than fetching the origin again.
    pub(super) fn set_lazy_content(
        &self,
        txn: &mut Txn,
        mxc: &str,
        content_type: Option<&str>,
        content_disposition: Option<&str>,
        content: &[u8],
    ) -> Result {
        let value = LazyContentRef {
            content_type,
            content_disposition,
            content,
        };

        let value = serialize_to_vec(Cbor(&value))?;

        txn.insert(&self.mediaid_lazycontent, mxc, value);

        Ok(())
    }

    #[cfg(feature = "url_preview")]
    /// The staged bytes a preview seeded for a lazy media mxc, if any.
    pub(super) async fn get_lazy_content(&self, mxc: &str) -> Result<Media> {
        self.mediaid_lazycontent
            .get(mxc)
            .await
            .deserialized::<Cbor<LazyContent>>()
            .map(at!(0))
            .map(Into::into)
    }

    #[cfg(feature = "url_preview")]
    /// Drops staged bytes, which promoting them into the media store finishes
    /// with — and which is also how a registration with no bytes is cleared.
    pub(super) fn remove_lazy_content(&self, txn: &mut Txn, mxc: &str) {
        txn.remove(&self.mediaid_lazycontent, mxc);
    }
}
