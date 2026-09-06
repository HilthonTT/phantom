use std::sync::Arc;

use phantom_core::{Result, at, err};
use phantom_database::{Cbor, Database, Deserialized, Map};
use ruma::http_headers::ContentDisposition;
use serde::Deserialize;

use crate::media::preview::CachedPreview;

use super::Media;

pub(crate) struct Data {
    pub mediaid_file: Arc<Map>,
    pub mediaid_lazy: Arc<Map>,
    pub mediaid_lazycontent: Arc<Map>,
    pub mediaid_pending: Arc<Map>,
    pub mediaid_user: Arc<Map>,
    pub url_preview: Arc<Map>,

    pub db: Arc<Database>,
}

#[derive(Debug)]
pub struct Metadata {
    pub content_disposition: Option<ContentDisposition>,
    pub content_type: Option<String>,
    pub(super) key: Vec<u8>,
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
#[derive(Deserialize)]
struct LazyContent {
    content_type: Option<String>,
    content_disposition: Option<String>,
    #[serde(with = "serde_bytes")]
    content: Vec<u8>,
}

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
            db: db.clone(),
            mediaid_file: db["mediaid_file"].clone(),
            mediaid_lazy: db["mediaid_lazy"].clone(),
            mediaid_lazycontent: db["mediaid_lazycontent"].clone(),
            mediaid_pending: db["mediaid_pending"].clone(),
            mediaid_user: db["mediaid_user"].clone(),
            url_preview: db["url_preview"].clone(),
        }
    }

    pub(super) async fn get_url_preview(&self, url: &str) -> Result<CachedPreview> {
        self.url_preview
            .get(url)
            .await
            .deserialized::<Cbor<_>>()
            .map(at!(0))
            .ok()
            .filter(CachedPreview::is_valid)
            .ok_or(err!(Request(NotFound("Expired from cache"))))
    }
}
