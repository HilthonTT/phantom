//! Backfills the `userid_deactivated` marker for accounts deactivated before
//! it existed.
//!
//! Until the marker, an account was deactivated when its password was empty.
//! But guests, appservice users registered without a password and the server
//! user have an empty password too, and nothing recorded which of them was
//! which: a guest is registered under a random localpart, exactly like an
//! account registered without a username, and carries no marker of its own.
//! So an empty password alone cannot be trusted, and this step marks only
//! what deactivation visibly did to an account:
//!
//! - the password is empty, and
//! - every device is gone, and
//! - the profile's display name is gone, and
//! - the account is joined to no room,
//!
//! because `full_deactivate` does all four, while registration (guest or
//! not) sets a display name and, unless login was inhibited, a device. The
//! server user and every user an appservice's namespace covers are skipped
//! outright, since those are passwordless by design.
//!
//! The rule errs towards leaving an account unmarked. A guest that logged out
//! and cleared its own name is marked, which costs it nothing: with no
//! password and no device it could never sign in again anyway. A deactivated
//! account that was somehow left with a device or a name is not marked, and
//! an admin can deactivate it again to set the marker.

use futures::StreamExt;
use phantom_core::{Result, info};
use ruma::OwnedUserId;

use crate::Services;

pub(super) async fn mark(services: &Services) -> Result<bool> {
    // The registrations are loaded by the appservice worker, which has not
    // started yet: migrations run first.
    let appservices = services.appservice.load_from_db().await?;
    let server_user = &services.server_state.server_user;

    let candidates: Vec<OwnedUserId> = services
        .users
        .stream()
        .filter(|user_id| {
            let skip = !services.server_state.user_is_local(user_id)
                || *user_id == server_user
                || appservices.values().any(|info| info.is_user_match(user_id));

            async move { !skip }
        })
        .map(ToOwned::to_owned)
        .collect()
        .await;

    let mut marked = 0_usize;
    for user_id in &candidates {
        services.server.check_running()?;

        if was_deactivated(services, user_id).await {
            services.users.mark_deactivated(user_id, None)?;
            marked = marked.saturating_add(1);
        }
    }

    info!(
        %marked,
        examined = candidates.len(),
        "Marked accounts deactivated before the deactivation marker existed"
    );

    Ok(true)
}

/// Whether an account bears every trace `full_deactivate` leaves.
async fn was_deactivated(services: &Services, user_id: &OwnedUserId) -> bool {
    let passwordless = services
        .users
        .password_hash(user_id)
        .await
        .is_ok_and(|hash| hash.is_empty());

    passwordless
        && services
            .users
            .all_device_ids(user_id)
            .next()
            .await
            .is_none()
        && services.profile.displayname(user_id).await.is_err()
        && services
            .rooms
            .state_cache
            .rooms_joined(user_id)
            .next()
            .await
            .is_none()
}
