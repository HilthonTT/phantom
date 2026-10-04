use std::collections::BTreeSet;

use futures::StreamExt;
use phantom_core::{Result, stream::BroadbandExt, stream::IterStream};
use phantom_service::Services;
use ruma::{
    OwnedUserId,
    api::client::sync::sync_events::v5::response::{Profiles, Room as ResponseRoom},
    events::{StateEventType, room::member::MembershipState},
    profile::{ProfileFieldName, UserProfileChanges, UserProfileUpdate},
    serde::Raw,
};

use super::{
    super::{range::Results, rooms::merged_room_details},
    Connection, SyncInfo, Window, selector,
};
use crate::client::profile::local_profile;

/// Collects the MSC4262 profiles extension payload.
///
/// phantom keeps no profile change log, so the extension carries the whole
/// profile of every user appearing in an initial room payload, plus the
/// syncing user's own when it is owed; later profile changes reach clients
/// through the member events in the timeline instead.
#[tracing::instrument(name = "profiles", level = "trace", skip_all)]
pub(super) async fn collect(
    SyncInfo {
        services,
        sender_user,
        ..
    }: SyncInfo<'_>,
    conn: &Connection,
    window: &Window,
    ranges: &Results,
) -> Result<Profiles> {
    let requested = conn.extensions.profiles.fields.as_deref();
    if requested.is_some_and(<[_]>::is_empty) {
        return Ok(Profiles::default());
    }

    let mut subjects = room_bases(services, conn, window, ranges).await;
    if conn.own_profile_owed() {
        subjects.insert(sender_user.to_owned());
    }

    let mut profiles = Profiles::default();
    profiles.users = subjects
        .into_iter()
        .stream()
        .broad_filter_map(async |user_id| {
            read_update(services, &user_id, requested)
                .await
                .map(|update| (user_id, update))
        })
        .collect()
        .await;

    Ok(profiles)
}

#[tracing::instrument(level = "trace", skip_all)]
async fn room_bases(
    services: &Services,
    conn: &Connection,
    window: &Window,
    ranges: &Results,
) -> BTreeSet<OwnedUserId> {
    let config = &conn.extensions.profiles;
    let owed = conn.profiles_fields_owed();

    let rooms: Vec<_> = selector(
        conn,
        window,
        config.lists.as_ref().map(|lists| lists.iter()),
        config.rooms.as_ref().map(|rooms| rooms.iter()),
    )
    .filter_map(|room_id| ranges.payload(room_id).map(|room| (room_id, room)))
    .filter(|(_, room)| room.initial.unwrap_or(false) || owed)
    .collect();

    let mut bases = BTreeSet::new();
    for (room_id, room) in rooms {
        bases.extend(subjects(room));

        let Some(selected) = window.get(room_id) else {
            continue;
        };

        let (_, state) = merged_room_details(conn, &selected.lists, room_id);
        let lazy = state
            .iter()
            .any(|(kind, key)| kind == &StateEventType::RoomMember && key == "$LAZY");

        let full = state.iter().any(|(kind, key)| {
            (kind == &StateEventType::RoomMember || kind == &StateEventType::from("*"))
                && key == "*"
        });

        if (lazy && !full) || selected.membership != Some(MembershipState::Join) {
            continue;
        }

        let members: Vec<OwnedUserId> = services
            .rooms
            .state_cache
            .room_members(room_id)
            .map(ToOwned::to_owned)
            .collect()
            .await;

        bases.extend(members);
    }

    bases
}

fn subjects(room: &ResponseRoom) -> impl Iterator<Item = OwnedUserId> + '_ {
    let senders = room
        .timeline
        .iter()
        .filter_map(|event| event.get_field("sender").ok().flatten());

    let members = room
        .timeline
        .iter()
        .filter_map(member)
        .chain(room.required_state.iter().filter_map(member));

    let heroes = room
        .heroes
        .iter()
        .flatten()
        .map(|hero| hero.user_id.clone());

    senders.chain(members).chain(heroes)
}

fn member<T>(event: &Raw<T>) -> Option<OwnedUserId> {
    event
        .get_field("type")
        .ok()
        .flatten()
        .filter(|kind: &StateEventType| kind == &StateEventType::RoomMember)
        .and_then(|_| event.get_field("state_key").ok().flatten())
}

/// The user's requested fields, or `None` when there are none to report.
async fn read_update(
    services: &Services,
    user_id: &OwnedUserId,
    requested: Option<&[ProfileFieldName]>,
) -> Option<UserProfileUpdate> {
    let profile = local_profile(services, user_id).await;
    let mut changes = UserProfileChanges::new();

    match requested {
        None => {
            for (name, value) in profile {
                changes.updated.insert(name.into(), value);
            }
        }
        Some(fields) => {
            for name in fields {
                match profile.get(name.as_str()) {
                    Some(value) => {
                        changes.updated.insert(name.clone(), value.clone());
                    }
                    None => changes.removed.push(name.clone()),
                }
            }
        }
    }

    (!changes.updated.is_empty() || !changes.removed.is_empty())
        .then_some(UserProfileUpdate::Updated(changes))
}
