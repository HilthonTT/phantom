mod data;
mod preview;
mod remote;
mod thumbnail;
#[cfg(feature = "media_thumbnail")]
mod video;

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime},
};

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use data::Data;
use futures::StreamExt;
use phantom_core::{
    Err, Error, Result, debug, err, http::StatusCode, implement, info, server::Server,
    stream::TryIgnore, sync::MutexMap, time::now_millis, warn,
};
use phantom_database::{Cbor, Deserialized, Interfix, serialize_to_vec};
use ruma::{
    MxcUri, OwnedMxcUri, OwnedUserId, ServerName, UserId,
    api::error::{ErrorKind, LimitExceededErrorData, RetryAfter},
    http_headers::ContentDisposition,
};
use serde::{Deserialize, Serialize};
#[cfg(feature = "media_thumbnail")]
use tokio::sync::Semaphore;
use tokio::{fs, sync::Notify};

pub use self::thumbnail::Dim;
#[cfg(feature = "media_thumbnail")]
use self::video::{FAILURES, Failures, sweep_staging_dir};
use crate::{Dep, client, config, moderation, server_state};

pub const MXC_LENGTH: usize = 32;

pub struct Service {
    path: PathBuf,
    services: Services,
    db: Data,
    url_preview_mutex: MutexMap<String, ()>,
    federation_mutex: MutexMap<String, ()>,
    mxc_state: MXCState,

    #[cfg(feature = "media_thumbnail")]
    video_thumbnail_slots: Semaphore,

    #[cfg(feature = "media_thumbnail")]
    video_thumbnail_failures: Mutex<Failures>,
}

struct Services {
    client: Dep<client::Service>,

    config: Dep<config::Service>,
    federation: Dep<crate::federation::Service>,
    moderation: Dep<moderation::Service>,
    server: Arc<Server>,
    server_state: Dep<server_state::Service>,
}

struct MXCState {
    notifiers: Mutex<HashMap<OwnedMxcUri, Arc<Notify>>>,

    ratelimiter: Mutex<HashMap<OwnedUserId, (Instant, f64)>>,
}

#[derive(Debug)]
pub struct Media {
    pub content: Vec<u8>,
    pub content_type: Option<String>,
    pub content_disposition: Option<ContentDisposition>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct FileMeta {
    pub content_type: Option<String>,

    pub content_disposition: Option<String>,

    pub size: u64,

    pub created: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Dimensions {
    pub width: u32,
    pub height: u32,
}

impl Dimensions {
    pub const ORIGINAL: Self = Self {
        width: 0,
        height: 0,
    };
}

#[async_trait]
impl crate::Service for Service {
    fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
        #[cfg(feature = "media_thumbnail")]
        sweep_staging_dir(&args.server.config);

        Ok(Arc::new(Self {
            path: args.server.config.media_path(),
            services: Services {
                client: args.depend::<client::Service>("client"),
                config: args.depend::<config::Service>("config"),
                federation: args.depend::<crate::federation::Service>("federation"),
                moderation: args.depend::<moderation::Service>("moderation"),
                server: args.server.clone(),
                server_state: args.depend::<server_state::Service>("server_state"),
            },
            url_preview_mutex: MutexMap::new(),
            federation_mutex: MutexMap::new(),
            mxc_state: MXCState {
                notifiers: Mutex::new(HashMap::new()),
                ratelimiter: Mutex::new(HashMap::new()),
            },
            #[cfg(feature = "media_thumbnail")]
            video_thumbnail_slots: Semaphore::new(
                args.server
                    .config
                    .media
                    .media_video_thumbnail_concurrency
                    .max(1),
            ),
            #[cfg(feature = "media_thumbnail")]
            video_thumbnail_failures: Mutex::new(Failures::new(FAILURES)),
            db: Data::new(args.db),
        }))
    }

    async fn worker(self: Arc<Self>) -> Result<()> {
        fs::create_dir_all(&self.path).await.map_err(|e| {
            err!(Config(
                "media_path",
                "Could not create the media directory {:?}: {e}",
                self.path
            ))
        })?;

        if self.services.server.config.media.media_startup_check {
            self.check().await?;
        }

        Ok(())
    }

    fn name(&self) -> &str {
        crate::make_name(std::module_path!())
    }
}

#[implement(Service)]
pub async fn create(
    &self,
    mxc: &MxcUri,
    uploader: Option<&UserId>,
    content_disposition: Option<&ContentDisposition>,
    content_type: Option<&str>,
    file: &[u8],
) -> Result {
    self.create_at(
        mxc,
        Dimensions::ORIGINAL,
        uploader,
        content_disposition,
        content_type,
        file,
    )
    .await
}

#[implement(Service)]
pub(super) async fn create_at(
    &self,
    mxc: &MxcUri,
    dimensions: Dimensions,
    uploader: Option<&UserId>,
    content_disposition: Option<&ContentDisposition>,
    content_type: Option<&str>,
    file: &[u8],
) -> Result {
    let key = self.key(mxc, dimensions)?;

    fs::write(self.file_path(&key), file)
        .await
        .map_err(|e| err!(Database("Could not write media {mxc}: {e}")))?;

    let meta = FileMeta {
        content_type: content_type.map(ToOwned::to_owned),
        content_disposition: content_disposition.map(ToString::to_string),
        size: file.len() as u64,
        created: now(),
    };

    self.db.mediaid_file.raw_put(&key, Cbor(&meta))?;

    if let Some(uploader) = uploader {
        let (server_name, media_id) = parts(mxc)?;

        self.db
            .mediaid_user
            .put_raw((server_name, media_id), uploader.as_bytes())?;
    }

    debug!(%mxc, size = file.len(), "Stored media");

    Ok(())
}

#[implement(Service)]
pub async fn create_pending(&self, mxc: &MxcUri, uploader: &UserId) -> Result<u64> {
    self.spend_reservation(uploader)?;

    let config = &self.services.config;
    let now = now_millis();
    let (reserved, earliest) = self.db.count_pending_for(uploader, now).await;

    if reserved >= config.media.max_pending_media_uploads {
        let retry_after = Duration::from_millis(earliest.saturating_sub(now));

        let mut data = LimitExceededErrorData::new();
        data.retry_after = Some(RetryAfter::Delay(retry_after));

        return Err(Error::Request(
            ErrorKind::LimitExceeded(data),
            "You have reserved as many media ids as you may hold unfilled.".into(),
            StatusCode::TOO_MANY_REQUESTS,
        ));
    }

    let lifetime = config
        .media
        .media_create_unused_expiration_time
        .saturating_mul(1000);

    let expires_at = now.saturating_add(lifetime);

    self.db.insert_pending(mxc, uploader, expires_at)?;

    Ok(expires_at)
}

#[implement(Service)]
pub async fn upload_pending(
    &self,
    mxc: &MxcUri,
    uploader: &UserId,
    content_disposition: Option<&ContentDisposition>,
    content_type: Option<&str>,
    file: &[u8],
) -> Result {
    let Ok((owner, expires_at)) = self.db.search_pending(mxc).await else {
        if self.exists(mxc, Dimensions::ORIGINAL).await {
            return Err!(Request(CannotOverwriteMedia(
                "Media {mxc} has been uploaded already."
            )));
        }

        return Err!(Request(NotFound("Media {mxc} was never reserved.")));
    };

    if owner != uploader {
        return Err!(Request(Forbidden("You did not reserve media {mxc}.")));
    }

    if expires_at < now_millis() {
        return Err!(Request(NotFound("The reservation of media {mxc} expired.")));
    }

    self.create(mxc, Some(uploader), content_disposition, content_type, file)
        .await?;

    self.db.remove_pending(mxc)?;

    let notifier = self.mxc_state.notifiers.lock()?.remove(mxc);

    if let Some(notifier) = notifier {
        notifier.notify_waiters();
    }

    Ok(())
}

#[implement(Service)]
pub async fn await_pending(&self, mxc: &MxcUri, timeout: Duration) -> Result {
    match self.db.search_pending(mxc).await {
        Ok((_, expires_at)) if expires_at > now_millis() => (),
        _ => return Err!(Request(NotFound("Media {mxc} is not being uploaded."))),
    }

    let notifier = self
        .mxc_state
        .notifiers
        .lock()?
        .entry(mxc.to_owned())
        .or_insert_with(|| Arc::new(Notify::new()))
        .clone();

    let notified = notifier.notified();
    tokio::pin!(notified);

    notified.as_mut().enable();

    if self.exists(mxc, Dimensions::ORIGINAL).await {
        return Ok(());
    }

    let filled = tokio::time::timeout(timeout, notified).await;

    if let Ok(mut notifiers) = self.mxc_state.notifiers.lock()
        && notifiers
            .get(mxc)
            .is_some_and(|notifier| Arc::strong_count(notifier) <= 2)
    {
        notifiers.remove(mxc);
    }

    filled.map_err(|_| {
        err!(Request(NotYetUploaded(
            "Media {mxc} has not been uploaded yet."
        )))
    })
}

#[implement(Service)]
fn spend_reservation(&self, uploader: &UserId) -> Result {
    let config = &self.services.config;
    let rate = f64::from(config.media.media_rc_create_per_second);
    let burst = f64::from(config.media.media_rc_create_burst_count);

    if rate <= 0.0 || burst <= 0.0 {
        return Ok(());
    }

    let now = Instant::now();
    let mut ratelimiter = self.mxc_state.ratelimiter.lock()?;

    let (spent_at, allowance) = ratelimiter
        .entry(uploader.to_owned())
        .or_insert_with(|| (now, burst));

    let elapsed = now.duration_since(*spent_at).as_secs_f64();
    let refilled = elapsed.mul_add(rate, *allowance).min(burst);

    if refilled < 1.0 {
        return Err(Error::Request(
            ErrorKind::LimitExceeded(LimitExceededErrorData::new()),
            "You are reserving media ids too quickly.".into(),
            StatusCode::TOO_MANY_REQUESTS,
        ));
    }

    *spent_at = now;
    *allowance = refilled - 1.0;

    Ok(())
}

#[implement(Service)]
pub async fn get(&self, mxc: &MxcUri, dimensions: Dimensions) -> Result<(FileMeta, Vec<u8>)> {
    let key = self.key(mxc, dimensions)?;

    let meta: FileMeta = self
        .db
        .mediaid_file
        .get(&key)
        .await
        .deserialized::<Cbor<FileMeta>>()
        .map(|Cbor(meta)| meta)?;

    let file = fs::read(self.file_path(&key)).await.map_err(|e| {
        err!(Database(warn!(
            "Media {mxc} is recorded but its file is missing: {e}"
        )))
    })?;

    Ok((meta, file))
}

#[implement(Service)]
pub async fn exists(&self, mxc: &MxcUri, dimensions: Dimensions) -> bool {
    let Ok(key) = self.key(mxc, dimensions) else {
        return false;
    };

    self.db.mediaid_file.get(&key).await.is_ok()
}

#[implement(Service)]
pub async fn delete(&self, mxc: &MxcUri) -> Result {
    let (server_name, media_id) = parts(mxc)?;
    let prefix = serialize_to_vec((server_name, media_id, Interfix))?;

    let keys: Vec<Vec<u8>> = self
        .db
        .mediaid_file
        .raw_keys_prefix(&prefix)
        .ignore_err()
        .map(<[u8]>::to_vec)
        .collect()
        .await;

    #[cfg(feature = "url_preview")]
    let had_lazy = self.forget_lazy_media(mxc.as_str()).await?;
    #[cfg(not(feature = "url_preview"))]
    let had_lazy = false;

    if keys.is_empty() {
        if had_lazy {
            return Ok(());
        }

        return Err!(Request(NotFound("Media {mxc} is not stored here.")));
    }

    for key in keys {
        if let Err(e) = fs::remove_file(self.file_path(&key)).await {
            debug!(%mxc, "Could not remove the media file: {e}");
        }

        self.db.mediaid_file.remove(&key)?;
    }

    self.db
        .mediaid_user
        .remove(&serialize_to_vec((server_name, media_id))?)?;

    Ok(())
}

#[implement(Service)]
pub async fn delete_from_server(&self, server_name: &ServerName) -> Result<usize> {
    if self.services.server_state.server_is_ours(server_name) {
        return Err!(Request(InvalidParam(
            "Refusing to purge this server's own media; delete it by id instead."
        )));
    }

    let mut removed: usize = 0;

    for mxc in self.media_of(server_name).await {
        match self.delete(&mxc).await {
            Ok(()) => removed = removed.saturating_add(1),
            Err(e) => debug!(%mxc, "Could not remove media: {e}"),
        }
    }

    info!(%server_name, removed, "Purged remote media");

    Ok(removed)
}

#[implement(Service)]
pub async fn media_of(&self, server_name: &ServerName) -> Vec<OwnedMxcUri> {
    let Ok(prefix) = serialize_to_vec((server_name, Interfix)) else {
        return Vec::new();
    };

    let keys: Vec<Vec<u8>> = self
        .db
        .mediaid_file
        .raw_keys_prefix(&prefix)
        .ignore_err()
        .map(<[u8]>::to_vec)
        .collect()
        .await;

    let mut media: Vec<OwnedMxcUri> = keys
        .iter()
        .filter_map(|key| media_id_of(key))
        .map(|media_id| OwnedMxcUri::from(format!("mxc://{server_name}/{media_id}")))
        .collect();

    media.sort_unstable();
    media.dedup();

    media
}

#[implement(Service)]
pub async fn uploader(&self, mxc: &MxcUri) -> Result<OwnedUserId> {
    let (server_name, media_id) = parts(mxc)?;

    self.db
        .mediaid_user
        .qry(&(server_name, media_id))
        .await
        .deserialized()
}

#[implement(Service)]
pub async fn check(&self) -> Result {
    let keys: Vec<Vec<u8>> = self
        .db
        .mediaid_file
        .raw_keys()
        .ignore_err()
        .map(<[u8]>::to_vec)
        .collect()
        .await;

    let mut missing: usize = 0;

    for key in &keys {
        if !fs::try_exists(self.file_path(key)).await.unwrap_or(false) {
            missing = missing.saturating_add(1);
        }
    }

    if missing > 0 {
        warn!(
            missing,
            total = keys.len(),
            "Media files are recorded but not present on disk. Restore them from a backup, or \
             purge the records with the admin media commands."
        );
    } else {
        debug!(total = keys.len(), "Media store checked");
    }

    Ok(())
}

#[implement(Service)]
fn key(&self, mxc: &MxcUri, dimensions: Dimensions) -> Result<Vec<u8>> {
    let (server_name, media_id) = parts(mxc)?;

    serialize_to_vec((server_name, media_id, dimensions.width, dimensions.height))
}

#[implement(Service)]
fn file_path(&self, key: &[u8]) -> PathBuf {
    let digest = phantom_core::hash::sha256::hash(key);

    self.path.join(URL_SAFE_NO_PAD.encode(digest))
}

fn parts(mxc: &MxcUri) -> Result<(&ServerName, &str)> {
    mxc.parts()
        .map_err(|e| err!(Request(InvalidParam("Invalid mxc URI {mxc}: {e}"))))
}

fn media_id_of(key: &[u8]) -> Option<String> {
    let mut parts = key.split(|byte| *byte == phantom_database::SEP);

    parts.next()?;

    std::str::from_utf8(parts.next()?).ok().map(str::to_owned)
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or_default()
}
