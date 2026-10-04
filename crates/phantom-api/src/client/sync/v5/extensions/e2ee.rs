use std::collections::HashSet;

use futures::{FutureExt, StreamExt, future::join};
use phantom_core::{
    Result, error,
    matrix::{Event, PduCount},
    stream::{BroadbandExt, IterStream, ReadyExt},
};
use ruma::{
    OwnedUserId, RoomId,
    api::client::sync::sync_events::{DeviceLists, v5::response},
    events::{
        StateEventType, TimelineEventType,
        room::member::{MembershipState, RoomMemberEventContent},
    },
};

use super::{super::rooms::shortstatehash_at, Connection, SyncInfo};
use crate::client::sync::share_encrypted_room;

type ChangedAndLeft = (HashSet<OwnedUserId>, HashSet<OwnedUserId>);

#[tracing::instrument(name = "e2ee", level = "trace", skip_all)]
pub(super) async fn collect(sync_info: SyncInfo<'_>, conn: &Connection) -> Result<response::E2EE> {
    let SyncInfo {
        services,
        sender_user,
        sender_device,
        ..
    } = sync_info;
    let Some(sender_device) = sender_device else {
        return Ok(response::E2EE::default());
    };

    let keys_changed: HashSet<_> = services
        .users
        .keys_changed(sender_user, conn.globalsince, Some(conn.next_batch))
        .map(ToOwned::to_owned)
        .collect()
        .await;

    let (changed, left) = services
        .rooms
        .state_cache
        .rooms_joined(sender_user)
        .map(ToOwned::to_owned)
        .broad_filter_map(async |room_id| collect_room(sync_info, conn, &room_id).await.ok())
        .ready_fold(
            (keys_changed, HashSet::new()),
            |(mut changed, mut left), room| {
                changed.extend(room.0);
                left.extend(room.1);
                (changed, left)
            },
        )
        .await;

    let left = left
        .into_iter()
        .stream()
        .filter_map(async |user_id| {
            (!share_encrypted_room(services, sender_user, &user_id, None).await).then_some(user_id)
        })
        .collect();

    let device_one_time_keys_count = async {
        let since = services.users.last_one_time_keys_update(sender_user).await;

        if since > conn.globalsince {
            services
                .users
                .count_one_time_keys(sender_user, sender_device)
                .await
        } else {
            Default::default()
        }
    };

    let (left, device_one_time_keys_count) = join(left, device_one_time_keys_count).boxed().await;

    // phantom stores no fallback keys; leaving the unused fallback key types
    // out tells clients the server lacks the feature, so they don't upload a
    // new fallback key on every sync.
    let mut e2ee = response::E2EE::default();
    e2ee.device_one_time_keys_count = device_one_time_keys_count;
    e2ee.device_lists = DeviceLists::new();
    e2ee.device_lists.changed = changed.into_iter().collect();
    e2ee.device_lists.left = left;

    Ok(e2ee)
}

#[tracing::instrument(level = "trace", skip_all, fields(room_id), ret)]
async fn collect_room(
    SyncInfo {
        services,
        sender_user,
        ..
    }: SyncInfo<'_>,
    conn: &Connection,
    room_id: &RoomId,
) -> Result<ChangedAndLeft> {
    let current_shortstatehash = services
        .rooms
        .state
        .get_room_shortstatehash(room_id)
        .map(|result| result.inspect_err(|e| error!("Room {room_id} has no state: {e}")));

    let room_keys_changed = services
        .users
        .room_keys_changed(room_id, conn.globalsince, Some(conn.next_batch))
        .map(|(user_id, _)| user_id.to_owned())
        .collect::<HashSet<_>>();

    let (current_shortstatehash, device_list_changed) =
        join(current_shortstatehash, room_keys_changed)
            .boxed()
            .await;

    let lists = (device_list_changed, HashSet::new());
    let Ok(current_shortstatehash) = current_shortstatehash else {
        return Ok(lists);
    };

    if current_shortstatehash <= conn.globalsince {
        return Ok(lists);
    }

    let Some(since_shortstatehash) =
        shortstatehash_at(services, room_id, PduCount::Normal(conn.globalsince)).await
    else {
        return Ok(lists);
    };

    if since_shortstatehash == current_shortstatehash {
        return Ok(lists);
    }

    let state_accessor = &services.rooms.state_accessor;
    let encrypted_at = async |shortstatehash| {
        state_accessor
            .state_get_shortid(shortstatehash, &StateEventType::RoomEncryption, "")
            .await
            .is_ok()
    };

    let current_encrypted = encrypted_at(current_shortstatehash).await;

    if !current_encrypted
        && services
            .config
            .client
            .device_key_update_encrypted_rooms_only
    {
        return Ok(lists);
    }

    // phantom records no per-user join position, so the burst on the sender's
    // own join reads the join from the state that changed since the last sync.
    let newly_encrypted = current_encrypted && !encrypted_at(since_shortstatehash).await;

    let added: Vec<_> = state_accessor
        .state_added((since_shortstatehash, current_shortstatehash))
        .broad_filter_map(async |(_shortstatekey, shorteventid)| {
            let event_id: ruma::OwnedEventId = services
                .rooms
                .short
                .get_eventid_from_short(shorteventid)
                .await
                .ok()?;

            services.rooms.timeline.get_pdu(&event_id).await.ok()
        })
        .ready_filter(|event| *event.event_type() == TimelineEventType::RoomMember)
        .ready_filter_map(|event| {
            let content: RoomMemberEventContent = event.get_content().ok()?;
            let user_id: OwnedUserId = event.state_key()?.parse().ok()?;

            Some((content.membership, user_id))
        })
        .collect()
        .await;

    let joined_since_last_sync = added.iter().any(|(membership, user_id)| {
        *membership == MembershipState::Join && user_id == sender_user
    });

    let members_burst = joined_since_last_sync || newly_encrypted;

    let joined_members: Vec<_> = if members_burst {
        services
            .rooms
            .state_cache
            .room_members(room_id)
            .ready_filter(|&user_id| user_id != sender_user)
            .map(|user_id| (MembershipState::Join, user_id.to_owned()))
            .collect()
            .await
    } else {
        Vec::new()
    };

    added
        .into_iter()
        .filter(|(_, user_id)| user_id != sender_user)
        .chain(joined_members)
        .stream()
        .fold(
            lists,
            async |(mut changed, mut left), (membership, user_id)| {
                use MembershipState::*;

                match membership {
                    Join if !share_encrypted_room(
                        services,
                        sender_user,
                        &user_id,
                        Some(room_id),
                    )
                    .await =>
                    {
                        changed.insert(user_id);
                    }
                    Leave => {
                        left.insert(user_id);
                    }
                    _ => {}
                }

                (changed, left)
            },
        )
        .map(Ok)
        .boxed()
        .await
}
