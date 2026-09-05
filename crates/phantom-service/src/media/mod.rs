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
//! **Thumbnails are not generated here.** The endpoint that would serve one
//! needs an image decoder, which this workspace does not depend on; the
//! storage for them is in place — a thumbnail is an entry at the same media id
//! with a width and height — so what is missing is the decoding, not the
//! plumbing. Nothing here claims to have produced a thumbnail it has not.
//!
//! [`check`]: Service::check

mod remote;

use std::{path::PathBuf, sync::Arc, time::SystemTime};

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use futures::StreamExt;
use phantom_core::{
    Err, Result, debug, err, implement, info, server::Server, stream::TryIgnore, warn,
};
use phantom_database::{Cbor, Deserialized, Interfix, Map, serialize_to_vec};
use ruma::{
    MxcUri, OwnedMxcUri, OwnedUserId, ServerName, UserId, http_headers::ContentDisposition,
};
use serde::{Deserialize, Serialize};
use tokio::fs;

use crate::{Dep, moderation, server_state};

pub struct Service {
    path: PathBuf,
    services: Services,
    db: Data,
}

struct Services {
    federation: Dep<crate::federation::Service>,
    moderation: Dep<moderation::Service>,
    server: Arc<Server>,
    server_state: Dep<server_state::Service>,
}

struct Data {
    mediaid_file: Arc<Map>,
    mediaid_user: Arc<Map>,
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
        Ok(Arc::new(Self {
            path: args.server.config.media_path(),
            services: Services {
                federation: args.depend::<crate::federation::Service>("federation"),
                moderation: args.depend::<moderation::Service>("moderation"),
                server: args.server.clone(),
                server_state: args.depend::<server_state::Service>("server_state"),
            },
            db: Data {
                mediaid_file: args.db["mediaid_file"].clone(),
                mediaid_user: args.db["mediaid_user"].clone(),
            },
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
    let key = self.key(mxc, Dimensions::ORIGINAL)?;

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

    if keys.is_empty() {
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
