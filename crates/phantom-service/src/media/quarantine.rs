//! Quarantined media: files an admin has taken out of circulation.
//!
//! A quarantined file is answered as missing, its thumbnails with it, and is
//! never fetched again from the server that holds it. The record is kept by
//! mxc URI and independently of the file, so media that was never fetched can
//! be quarantined ahead of time, and deleting the file does not lift the
//! quarantine. The file itself is kept until an admin deletes it, so a
//! quarantine can be lifted without loss.

use futures::StreamExt;
use phantom_core::{
    Err, Result, implement, info,
    stream::{ReadyExt, TryIgnore},
    time::now_millis,
};
use phantom_database::Json;
use ruma::{MxcUri, OwnedMxcUri, OwnedUserId, UserId};
use serde::{Deserialize, Serialize};

use super::{Service, parts};

/// One quarantined mxc URI, as stored.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Quarantine {
    pub mxc: OwnedMxcUri,
    pub reason: Option<String>,
    pub by: Option<OwnedUserId>,
    pub at_ms: u64,
}

#[implement(Service)]
pub async fn is_quarantined(&self, mxc: &MxcUri) -> bool {
    self.db
        .mediaid_quarantine
        .exists(mxc.as_str())
        .await
        .is_ok()
}

/// Answers a quarantined file as missing. It is worded as for any file not
/// stored here, so a client cannot tell quarantine from absence.
#[implement(Service)]
pub(super) async fn refuse_quarantined(&self, mxc: &MxcUri) -> Result {
    if self.is_quarantined(mxc).await {
        return Err!(Request(NotFound("Media {mxc} is not stored here.")));
    }

    Ok(())
}

/// Quarantines one file. Answers false when it already was.
#[implement(Service)]
pub async fn quarantine(&self, mxc: &MxcUri, by: &UserId, reason: Option<&str>) -> Result<bool> {
    parts(mxc)?;

    if self.is_quarantined(mxc).await {
        return Ok(false);
    }

    let record = Quarantine {
        mxc: mxc.to_owned(),
        reason: reason.map(ToOwned::to_owned),
        by: Some(by.to_owned()),
        at_ms: now_millis(),
    };

    self.db
        .mediaid_quarantine
        .raw_put(mxc.as_str(), Json(&record))?;

    info!(%mxc, %by, "Quarantined media");

    Ok(true)
}

/// Quarantines every file given, answering with how many were not already.
/// A URI that is not a valid mxc URI is skipped.
#[implement(Service)]
pub async fn quarantine_all<I>(&self, mxcs: I, by: &UserId, reason: Option<&str>) -> usize
where
    I: IntoIterator<Item = OwnedMxcUri> + Send,
    I::IntoIter: Send,
{
    let mut added: usize = 0;
    for mxc in mxcs {
        if matches!(self.quarantine(&mxc, by, reason).await, Ok(true)) {
            added = added.saturating_add(1);
        }
    }

    added
}

/// Lifts a file's quarantine.
#[implement(Service)]
pub async fn unquarantine(&self, mxc: &MxcUri) -> Result {
    if !self.is_quarantined(mxc).await {
        return Err!(Request(NotFound("Media {mxc} is not quarantined.")));
    }

    self.db.mediaid_quarantine.remove(mxc.as_str())?;

    info!(%mxc, "Lifted the quarantine of media");

    Ok(())
}

/// Every quarantined file, oldest quarantine first.
#[implement(Service)]
pub async fn quarantined(&self) -> Vec<Quarantine> {
    let mut records: Vec<Quarantine> = self
        .db
        .mediaid_quarantine
        .raw_stream()
        .ignore_err()
        .filter_map(async |(_, value)| serde_json::from_slice(value).ok())
        .collect()
        .await;

    records.sort_by_key(|record| record.at_ms);
    records
}

/// Every file the user uploaded here, by the uploader record each upload
/// leaves.
#[implement(Service)]
pub async fn uploads_of(&self, user: &UserId) -> Vec<OwnedMxcUri> {
    // Both key parts are strings, but the key is split by hand to read it the
    // same way the media listing reads `mediaid_file`.
    self.db
        .mediaid_user
        .raw_stream()
        .ignore_err()
        .ready_filter_map(|(key, value)| {
            if value != user.as_bytes() {
                return None;
            }

            let mut parts = key.splitn(2, |byte| *byte == phantom_database::SEP);
            let server = std::str::from_utf8(parts.next()?).ok()?;
            let media_id = std::str::from_utf8(parts.next()?).ok()?;

            Some(OwnedMxcUri::from(format!("mxc://{server}/{media_id}")))
        })
        .collect()
        .await
}
