//! Clearing old media out of the store by age.
//!
//! The store records when a file was stored, not when it was last served, so
//! age is the only measure there is. For a remote copy that is harmless: it
//! is fetched again if anyone asks for it. A local upload has no other copy,
//! so purging those is the operator's call to make deliberately.

use phantom_core::{Result, debug, implement, info};
use serde::Serialize;

use super::Service;

/// Which side of the store a purge clears.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Origin {
    /// Copies of other servers' media.
    Remote,

    /// This server's own uploads.
    Local,
}

/// What a purge removed.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Purged {
    pub removed: usize,
    pub bytes: u64,
    pub failed: usize,
}

/// Deletes every file of the origin stored before `before_secs`, with its
/// thumbnails.
#[implement(Service)]
pub async fn purge_older_than(&self, origin: Origin, before_secs: u64) -> Result<Purged> {
    let mut purged = Purged::default();

    for media in self.list().await {
        let local = media
            .mxc
            .server_name()
            .is_ok_and(|server| self.services.server_state.server_is_ours(server));

        if local != (origin == Origin::Local) || media.meta.created >= before_secs {
            continue;
        }

        match self.delete(&media.mxc).await {
            Ok(()) => {
                purged.removed = purged.removed.saturating_add(1);
                purged.bytes = purged.bytes.saturating_add(media.meta.size);
            }
            Err(e) => {
                debug!(mxc = %media.mxc, "Could not purge media: {e}");
                purged.failed = purged.failed.saturating_add(1);
            }
        }
    }

    info!(?origin, before_secs, ?purged, "Purged media by age");

    Ok(purged)
}
