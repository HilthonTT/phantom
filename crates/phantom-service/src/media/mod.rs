//! Uploaded files, and the files fetched from other servers to serve them.
//!
//! Media is split between two stores, and the split is the whole design.
//! **The bytes are files on disk**, because they are large, written once and
//! read whole, which is what a filesystem is for. **What is known about them
//! is in the database**, because it is small, queried by several keys, and has
//! to be consistent with everything else the server knows. The two are backed
//! up and restored together or not at all: metadata pointing at a file that is
//! not there is what [`check`] exists to find.
//!
//! A file is named on disk by a hash of the database key that names it, so the
//! layout on disk carries no information of its own. That is deliberate. It
//! means a media id containing anything at all — a path separator, a Windows
//! device name, four thousand characters — cannot become a filename, and it
//! means the directory can be listed without leaking who uploaded what.
//!
//! **A thumbnail is media in its own right**: an entry at the same media id
//! with a width and a height, where the original is `(0, 0)`. That is what
//! makes serving one an ordinary media read, and what makes deleting a piece
//! of media take its thumbnails with it. Generating one needs an image
//! decoder, which is what the `media_thumbnail` feature carries; a build
//! without it answers with the original rather than claiming to have produced
//! something. See the `thumbnail` module.
//!
//! [`check`]: Service::check

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

/// Characters in a media id this server mints.
///
/// Long enough that a media id cannot be guessed, which is the only thing
/// keeping an unauthenticated download of somebody else's file out of reach.
pub const MXC_LENGTH: usize = 32;

pub struct Service {
    path: PathBuf,
    services: Services,
    db: Data,
    url_preview_mutex: MutexMap<String, ()>,
    federation_mutex: MutexMap<String, ()>,
    mxc_state: MXCState,

    /// How many frame extractions may run at once. Held from staging a video
    /// through to the program exiting, so it also bounds how much of the
    /// staging directory is in use.
    #[cfg(feature = "media_thumbnail")]
    video_thumbnail_slots: Semaphore,

    /// Videos the extraction program has already failed on, so that the next
    /// request for another size does not spend a slot reaching the same
    /// verdict.
    #[cfg(feature = "media_thumbnail")]
    video_thumbnail_failures: Mutex<Failures>,
}

struct Services {
    /// The outbound HTTP clients, for the URL preview fetches and the media
    /// a preview names.
    client: Dep<client::Service>,

    /// Derefs to the running config, so a reload is seen by the next preview
    /// rather than at the next restart.
    config: Dep<config::Service>,
    federation: Dep<crate::federation::Service>,
    moderation: Dep<moderation::Service>,
    server: Arc<Server>,
    server_state: Dep<server_state::Service>,
}

/// What is known about media ids that have been reserved but not yet filled.
///
/// A client may ask for a media id before it has the file, so that it can send
/// the message naming it and upload afterwards. Both halves here exist because
/// of that gap: the notifiers are how a download waiting on one is woken the
/// moment it is filled, and the allowances are what stops a client from
/// reserving ids faster than it could ever fill them.
struct MXCState {
    /// The waiters on each reserved media id, notified when it is filled.
    notifiers: Mutex<HashMap<OwnedMxcUri, Arc<Notify>>>,

    /// Each user's remaining reservation allowance, and when it was last
    /// spent. A token bucket: see [`Service::create_pending`].
    ratelimiter: Mutex<HashMap<OwnedUserId, (Instant, f64)>>,
}

#[derive(Debug)]
pub struct Media {
    pub content: Vec<u8>,
    pub content_type: Option<String>,
    pub content_disposition: Option<ContentDisposition>,
}

/// What is known about a stored file besides its bytes.
///
/// Stored as one value rather than packed into the key, so that a field can be
/// added without every existing key becoming unreadable — which matters here
/// more than usual, because the key is also what names the file on disk.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct FileMeta {
    /// The type the uploader claimed, which is what the download is served as
    /// after `content_disposition` has decided whether to serve it inline.
    pub content_type: Option<String>,

    /// The `Content-Disposition` header to serve, already decided and
    /// sanitised.
    pub content_disposition: Option<String>,

    /// The file's size in bytes, so a listing does not have to stat every file.
    pub size: u64,

    /// When the file was stored, as seconds since the epoch. Used by the admin
    /// commands that purge media by age.
    pub created: u64,
}

/// One stored file: which media it is and, for a thumbnail, at what size.
///
/// The full-size original is `(0, 0)`, which is not a size a thumbnail can
/// have and so cannot collide with one.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Dimensions {
    pub width: u32,
    pub height: u32,
}

impl Dimensions {
    /// The original, as opposed to any thumbnail of it.
    pub const ORIGINAL: Self = Self {
        width: 0,
        height: 0,
    };
}

#[async_trait]
impl crate::Service for Service {
    fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
        // Before anything can stage a file this would race, and before the
        // service graph exists to read the configuration through.
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
                args.server.config.media_video_thumbnail_concurrency.max(1),
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

        if self.services.server.config.media_startup_check {
            self.check().await?;
        }

        Ok(())
    }

    fn name(&self) -> &str {
        crate::make_name(std::module_path!())
    }
}

/// Stores a file and what is known about it.
///
/// Writing the file before the metadata is deliberate: a file with no metadata
/// is unreachable and will be found by [`check`], while metadata with no file
/// is a download that fails at the last moment. Neither is good, but only the
/// second is visible to a user.
///
/// [`check`]: Service::check
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

/// [`create`] at one size, which is how a thumbnail comes to be stored.
///
/// [`create`]: Service::create
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

/// Reserves a media id for a file the client does not have ready yet.
///
/// A client that has to upload before it can send the message naming the
/// upload must either hold the message back or guess the id. This is the
/// third answer (MSC2246): the id is minted now and filled later, so the
/// message can be sent immediately and the file can follow.
///
/// Returns when the reservation expires, in milliseconds since the epoch,
/// which is what the client is told so that it knows how long it has.
#[implement(Service)]
pub async fn create_pending(&self, mxc: &MxcUri, uploader: &UserId) -> Result<u64> {
    self.spend_reservation(uploader)?;

    let config = &self.services.config;
    let now = now_millis();
    let (reserved, earliest) = self.db.count_pending_for(uploader, now).await;

    if reserved >= config.max_pending_media_uploads {
        // Retry when the oldest of them expires: that is the first moment
        // this request could succeed, and a client told so does not poll.
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
        .media_create_unused_expiration_time
        .saturating_mul(1000);

    let expires_at = now.saturating_add(lifetime);

    self.db.insert_pending(mxc, uploader, expires_at)?;

    Ok(expires_at)
}

/// Fills a media id reserved earlier by [`create_pending`].
///
/// Only by the user who reserved it, and only before it expires. An expired
/// reservation is not renewed by filling it: the id stays unresolvable for
/// good, since something may already have been told it would resolve.
///
/// [`create_pending`]: Service::create_pending
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

    // Whoever is waiting on it is waiting on the file, which is now stored.
    let notifier = self.mxc_state.notifiers.lock()?.remove(mxc);

    if let Some(notifier) = notifier {
        notifier.notify_waiters();
    }

    Ok(())
}

/// Waits for a reserved media id to be filled, for as long as the client said
/// it was willing to wait.
///
/// What a download does when it finds nothing stored: a client that was sent
/// a message naming an id may well ask for it before the upload lands, and
/// answering "not found" straight away would show a broken picture for a file
/// that is seconds away. Returns as soon as the id is filled, and errors if
/// it was never reserved or the wait ran out.
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

    // Enrolled before the store is checked, so that an upload landing between
    // the two is caught by the wait rather than lost between them.
    notified.as_mut().enable();

    if self.exists(mxc, Dimensions::ORIGINAL).await {
        return Ok(());
    }

    let filled = tokio::time::timeout(timeout, notified).await;

    // The last waiter takes the notifier with it. A reservation nobody ever
    // fills is never notified, and so would otherwise leave an entry behind
    // for the life of the process.
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

/// Spends one of a user's media id reservations, refusing where there is none
/// left to spend.
///
/// A token bucket: the allowance refills at `media_rc_create_per_second` up to
/// `media_rc_create_burst_count`, and reserving costs one. Reserving is rate
/// limited apart from uploading because it is so much cheaper — a reservation
/// is a row, where an upload is a file — so a client that only reserves would
/// otherwise be limited by nothing.
#[implement(Service)]
fn spend_reservation(&self, uploader: &UserId) -> Result {
    let config = &self.services.config;
    let rate = f64::from(config.media_rc_create_per_second);
    let burst = f64::from(config.media_rc_create_burst_count);

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

/// Reads a stored file back, with what is known about it.
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

/// Whether the file is stored here, without reading it.
#[implement(Service)]
pub async fn exists(&self, mxc: &MxcUri, dimensions: Dimensions) -> bool {
    let Ok(key) = self.key(mxc, dimensions) else {
        return false;
    };

    self.db.mediaid_file.get(&key).await.is_ok()
}

/// Removes a file and everything recorded about it, thumbnails included.
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

    // A URI a URL preview minted has no file of its own until somebody
    // downloads it, so dropping the registration is the whole deletion.
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
        // The file first, for the same reason as in `create`: a file left
        // behind wastes space, where a record left behind serves a download
        // that then fails.
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

/// Removes every file cached from one remote server.
///
/// Returns how many were removed. Local media is refused rather than silently
/// skipped: a request to purge this server's own media names something the
/// caller almost certainly did not mean.
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

/// Every piece of media stored from one server.
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

/// Who uploaded a piece of local media, where it was one of our users.
#[implement(Service)]
pub async fn uploader(&self, mxc: &MxcUri) -> Result<OwnedUserId> {
    let (server_name, media_id) = parts(mxc)?;

    self.db
        .mediaid_user
        .qry(&(server_name, media_id))
        .await
        .deserialized()
}

/// Reports metadata whose file is missing.
///
/// Reporting rather than repairing: a missing file may mean a half-restored
/// backup, and deleting the record of it would turn a recoverable state into
/// an unrecoverable one.
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

/// The database key one stored file is recorded under.
#[implement(Service)]
fn key(&self, mxc: &MxcUri, dimensions: Dimensions) -> Result<Vec<u8>> {
    let (server_name, media_id) = parts(mxc)?;

    serialize_to_vec((server_name, media_id, dimensions.width, dimensions.height))
}

/// Where on disk the file for a key lives.
///
/// The name is a hash of the key rather than anything derived from the media
/// id: a media id is a string another server chose, and no string another
/// server chose should ever reach the filesystem.
#[implement(Service)]
fn file_path(&self, key: &[u8]) -> PathBuf {
    let digest = phantom_core::hash::sha256::hash(key);

    self.path.join(URL_SAFE_NO_PAD.encode(digest))
}

/// The server and media id of an `mxc://` URI.
fn parts(mxc: &MxcUri) -> Result<(&ServerName, &str)> {
    mxc.parts()
        .map_err(|e| err!(Request(InvalidParam("Invalid mxc URI {mxc}: {e}"))))
}

/// The media id out of a stored key, which begins with the server name.
fn media_id_of(key: &[u8]) -> Option<String> {
    let mut parts = key.split(|byte| *byte == phantom_database::SEP);

    parts.next()?;

    std::str::from_utf8(parts.next()?).ok().map(str::to_owned)
}

/// Now, as seconds since the epoch, or zero if the clock is before it.
fn now() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or_default()
}
