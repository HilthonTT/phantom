//! What a sliding sync connection carries from one request to the next.
//!
//! phantom's sync service keeps a connection's lists, subscriptions and
//! extension settings plus per-list maps of room positions. The handler works
//! on this richer view and folds it back into that store after each response:
//! each room's cursor and configuration fingerprint ride two reserved
//! `known_rooms` keys that no client list id can take, as list ids are
//! printable.

use std::collections::BTreeMap;

use phantom_service::{Services, accounts::sync};
use ruma::{
    OwnedDeviceId, OwnedRoomId, RoomId, UserId,
    api::client::sync::sync_events::v5::{
        Request, request,
        request::{AccountData, E2EE, Profiles, Receipts, ToDevice, Typing},
    },
    profile::ProfileFieldName,
};

use super::ListId;

/// `known_rooms` key holding each room's delivery cursor.
const ROOMSINCE_KEY: &str = "\0roomsince";

/// `known_rooms` key holding each room's delivered configuration fingerprint.
const CONFIG_KEY: &str = "\0config";

pub(super) type Subscriptions = BTreeMap<OwnedRoomId, request::RoomSubscription>;
pub(super) type Lists = BTreeMap<ListId, request::List>;
pub(super) type Rooms = BTreeMap<OwnedRoomId, Room>;
type RoomUpdate<'a> = (&'a RoomId, Option<u64>);

#[derive(Debug, Default)]
pub(super) struct Connection {
    pub(super) globalsince: u64,
    pub(super) next_batch: u64,
    pub(super) lists: Lists,
    pub(super) extensions: request::Extensions,
    pub(super) subscriptions: Subscriptions,
    pub(super) rooms: Rooms,

    /// Whether this pass owes the syncing user's whole profile to the MSC4262
    /// profiles extension: the extension is on and the connection is new, the
    /// extension was just switched on, or its field filter just widened.
    pub(super) own_profile_owed: bool,

    /// Whether the connection asked for a profile field it did not have before.
    pub(super) profiles_fields_widened: bool,
}

/// Delivery progress for one room on a Sliding Sync connection.
///
/// The cursor advances with complete ranges; configuration tracks room payloads.
#[derive(Clone, Debug, Default)]
pub(super) struct Room {
    pub(super) roomsince: u64,
    pub(super) config_hash: u64,
}

impl Connection {
    /// Rebuilds the handler's view from the stored connection.
    pub(super) fn load(stored: sync::Connection) -> Self {
        let sync::Connection {
            lists,
            subscriptions,
            mut known_rooms,
            extensions,
        } = stored;

        let config = known_rooms.remove(CONFIG_KEY).unwrap_or_default();
        let rooms = known_rooms
            .remove(ROOMSINCE_KEY)
            .unwrap_or_default()
            .into_iter()
            .map(|(room_id, roomsince)| {
                let config_hash = config.get(&room_id).copied().unwrap_or_default();
                (
                    room_id,
                    Room {
                        roomsince,
                        config_hash,
                    },
                )
            })
            .collect();

        Self {
            lists,
            subscriptions,
            extensions,
            rooms,
            ..Default::default()
        }
    }

    /// Replaces the stored connection with this one.
    pub(super) fn store(
        &self,
        services: &Services,
        user_id: &UserId,
        device_id: &OwnedDeviceId,
        conn_id: Option<&str>,
    ) {
        let sync = &services.sync;

        sync.forget(user_id, device_id, conn_id);
        sync.remember(
            user_id,
            device_id,
            conn_id,
            self.lists.clone(),
            self.subscriptions.clone(),
            self.extensions.clone(),
        );

        let rooms = || {
            self.rooms
                .iter()
                .map(|(room_id, room)| (room_id.clone(), room))
        };

        sync.remember_rooms(
            user_id,
            device_id,
            conn_id,
            ROOMSINCE_KEY,
            rooms().map(|(room_id, room)| (room_id, room.roomsince)),
        );

        sync.remember_rooms(
            user_id,
            device_id,
            conn_id,
            CONFIG_KEY,
            rooms().map(|(room_id, room)| (room_id, room.config_hash)),
        );
    }

    #[tracing::instrument(level = "debug", skip(self))]
    pub(super) fn update_rooms_prologue(&mut self, retard_since: Option<u64>) {
        self.rooms.values_mut().for_each(|room| {
            if let Some(retard_since) = retard_since
                && room.roomsince > retard_since
            {
                room.roomsince = retard_since;
                room.config_hash = 0;
            }
        });
    }

    /// Advance the per-room cursor for each complete bounded room range.
    ///
    /// `roomsince` is the lower bound of every content query for its room. Only
    /// rooms whose complete range was safely assembled may advance. A failed room
    /// keeps its cursor and retries the same range after a later wake.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(super) fn update_rooms_epilogue<'a, Complete>(&mut self, complete: Complete)
    where
        Complete: Iterator<Item = RoomUpdate<'a>> + Send + 'a,
    {
        let next_batch = self.next_batch;
        complete.for_each(|(room_id, config)| {
            let room = self.rooms.entry(room_id.into()).or_default();

            room.roomsince = next_batch;
            if let Some(config_hash) = config {
                room.config_hash = config_hash;
            }
        });
    }

    /// Whether the syncing user's whole profile is owed to the profiles extension.
    #[inline]
    #[must_use]
    pub(super) fn own_profile_owed(&self) -> bool {
        self.extensions.profiles.enabled.unwrap_or(false)
            && (self.own_profile_owed || self.globalsince == 0)
    }

    /// Whether a base is owed for a profile field the connection did not have
    /// before.
    #[inline]
    #[must_use]
    pub(super) fn profiles_fields_owed(&self) -> bool {
        self.profiles_fields_widened && self.own_profile_owed()
    }

    #[tracing::instrument(level = "debug", skip_all)]
    pub(super) fn update_cache(&mut self, request: &Request) -> bool {
        let lists_changed = Self::update_cache_lists(request, self);
        let subscriptions_changed = Self::update_cache_subscriptions(request, self);

        let was_enabled = self.extensions.profiles.enabled.unwrap_or(false);
        let fields_widened = Self::update_cache_extensions(request, self);
        let switched_on = !was_enabled && self.extensions.profiles.enabled.unwrap_or(false);

        self.profiles_fields_widened = fields_widened;
        self.own_profile_owed = fields_widened || switched_on;

        lists_changed || subscriptions_changed
    }

    fn update_cache_lists(request: &Request, cached: &mut Self) -> bool {
        request
            .lists
            .iter()
            .fold(false, |changed, (list_id, request_list)| {
                let list_changed = match cached.lists.get_mut(list_id) {
                    Some(cached_list) => Self::update_cache_list(request_list, cached_list),
                    None => {
                        cached.lists.insert(list_id.clone(), request_list.clone());

                        true
                    }
                };

                changed | list_changed
            })
    }

    fn update_cache_list(request: &request::List, cached: &mut request::List) -> bool {
        let ranges_changed = request.ranges != cached.ranges;
        let timeline_limit_changed =
            request.room_details.timeline_limit != cached.room_details.timeline_limit;

        let required_state_changed = !request.room_details.required_state.is_empty()
            && request.room_details.required_state != cached.room_details.required_state;

        let filters_changed = request.filters.as_ref().is_some_and(|request| {
            cached
                .filters
                .as_ref()
                .is_none_or(|cached| !list_filters_are_equal(request, cached))
        });

        let changed =
            ranges_changed || timeline_limit_changed || required_state_changed || filters_changed;

        if ranges_changed {
            cached.ranges.clone_from(&request.ranges);
        }

        cached.room_details.timeline_limit = request.room_details.timeline_limit;

        if required_state_changed {
            cached
                .room_details
                .required_state
                .clone_from(&request.room_details.required_state);
        }

        if filters_changed {
            cached.filters.clone_from(&request.filters);
        }

        changed
    }

    fn update_cache_subscriptions(request: &Request, cached: &mut Self) -> bool {
        let changed = !subscriptions_are_equal(&request.room_subscriptions, &cached.subscriptions);

        if changed {
            cached.subscriptions.clone_from(&request.room_subscriptions);
        }

        changed
    }

    /// Merges the request's extension settings into the connection.
    ///
    /// Returns whether the MSC4262 field filter named a field the connection did
    /// not have, which the profiles extension owes a base for.
    fn update_cache_extensions(request: &Request, cached: &mut Self) -> bool {
        let request = &request.extensions;
        let cached = &mut cached.extensions;

        update_cache_account_data(&request.account_data, &mut cached.account_data);
        update_cache_receipts(&request.receipts, &mut cached.receipts);
        update_cache_typing(&request.typing, &mut cached.typing);
        update_cache_to_device(&request.to_device, &mut cached.to_device);
        update_cache_e2ee(&request.e2ee, &mut cached.e2ee);

        update_cache_profiles(&request.profiles, &mut cached.profiles)
    }
}

fn subscriptions_are_equal(request: &Subscriptions, cached: &Subscriptions) -> bool {
    request.len() == cached.len()
        && request.iter().zip(cached).all(|(request, cached)| {
            request.0 == cached.0 && subscription_is_equal(request.1, cached.1)
        })
}

fn subscription_is_equal(
    request: &request::RoomSubscription,
    cached: &request::RoomSubscription,
) -> bool {
    request.timeline_limit == cached.timeline_limit
        && request.required_state == cached.required_state
}

fn list_filters_are_equal(request: &request::ListFilters, cached: &request::ListFilters) -> bool {
    request.is_dm == cached.is_dm
        && request.is_encrypted == cached.is_encrypted
        && request.is_invite == cached.is_invite
        && request.room_types == cached.room_types
        && request.not_room_types == cached.not_room_types
}

fn update_cache_account_data(request: &AccountData, cached: &mut AccountData) {
    some_or_sticky(request.enabled.as_ref(), &mut cached.enabled);
    some_or_sticky(request.lists.as_ref(), &mut cached.lists);
    some_or_sticky(request.rooms.as_ref(), &mut cached.rooms);
}

fn update_cache_receipts(request: &Receipts, cached: &mut Receipts) {
    some_or_sticky(request.enabled.as_ref(), &mut cached.enabled);
    some_or_sticky(request.rooms.as_ref(), &mut cached.rooms);
    some_or_sticky(request.lists.as_ref(), &mut cached.lists);
}

fn update_cache_typing(request: &Typing, cached: &mut Typing) {
    some_or_sticky(request.enabled.as_ref(), &mut cached.enabled);
    some_or_sticky(request.rooms.as_ref(), &mut cached.rooms);
    some_or_sticky(request.lists.as_ref(), &mut cached.lists);
}

fn update_cache_to_device(request: &ToDevice, cached: &mut ToDevice) {
    some_or_sticky(request.enabled.as_ref(), &mut cached.enabled);
    cached.since.clone_from(&request.since);
}

/// Merges the profiles extension settings into the connection.
///
/// Returns whether the request widened the field filter, which is read before
/// the merge overwrites the filter it compares against.
fn update_cache_profiles(request: &Profiles, cached: &mut Profiles) -> bool {
    some_or_sticky(request.enabled.as_ref(), &mut cached.enabled);
    some_or_sticky(request.rooms.as_ref(), &mut cached.rooms);
    some_or_sticky(request.lists.as_ref(), &mut cached.lists);

    // Compare against the cached filter before the merge below overwrites it.
    let widened = fields_widened(request.fields.as_deref(), cached.fields.as_deref());

    some_or_sticky(request.fields.as_ref(), &mut cached.fields);

    widened
}

/// Whether the request names a profile field the connection did not ask for.
///
/// An absent cached filter already covers every field, and an absent request
/// keeps the cached one, so neither widens anything.
fn fields_widened(
    request: Option<&[ProfileFieldName]>,
    cached: Option<&[ProfileFieldName]>,
) -> bool {
    request
        .zip(cached)
        .is_some_and(|(request, cached)| request.iter().any(|name| !cached.contains(name)))
}

fn update_cache_e2ee(request: &E2EE, cached: &mut E2EE) {
    some_or_sticky(request.enabled.as_ref(), &mut cached.enabled);
}

fn some_or_sticky<T: Clone>(target: Option<&T>, cached: &mut Option<T>) {
    if let Some(target) = target {
        cached.replace(target.clone());
    }
}

#[cfg(test)]
mod tests {
    use ruma::{
        api::client::sync::sync_events::v5::request::Profiles, owned_room_id,
        profile::ProfileFieldName,
    };

    use super::{Connection, Room, fields_widened, update_cache_profiles};

    #[test]
    fn retarding_rewinds_only_rooms_past_the_position() {
        let ahead = owned_room_id!("!ahead:example.com");
        let behind = owned_room_id!("!behind:example.com");
        let mut conn = Connection::default();

        conn.rooms.insert(
            ahead.clone(),
            Room {
                roomsince: 9,
                config_hash: 3,
            },
        );
        conn.rooms.insert(
            behind.clone(),
            Room {
                roomsince: 4,
                config_hash: 5,
            },
        );

        conn.update_rooms_prologue(Some(6));

        assert_eq!(conn.rooms[&ahead].roomsince, 6);
        assert_eq!(conn.rooms[&ahead].config_hash, 0);
        assert_eq!(conn.rooms[&behind].roomsince, 4);
        assert_eq!(conn.rooms[&behind].config_hash, 5);
    }

    #[test]
    fn epilogue_advances_complete_rooms_to_the_batch() {
        let room_id = owned_room_id!("!room:example.com");
        let mut conn = Connection {
            next_batch: 12,
            ..Default::default()
        };

        conn.update_rooms_epilogue([(room_id.as_ref(), Some(7))].into_iter());

        assert_eq!(conn.rooms[&room_id].roomsince, 12);
        assert_eq!(conn.rooms[&room_id].config_hash, 7);

        conn.next_batch = 15;
        conn.update_rooms_epilogue([(room_id.as_ref(), None)].into_iter());

        assert_eq!(conn.rooms[&room_id].roomsince, 15);
        assert_eq!(conn.rooms[&room_id].config_hash, 7);
    }

    #[test]
    fn a_new_field_widens_the_filter() {
        let display = [ProfileFieldName::DisplayName];
        let both = [ProfileFieldName::DisplayName, ProfileFieldName::AvatarUrl];

        assert!(fields_widened(Some(&both), Some(&display)));
        assert!(!fields_widened(Some(&display), Some(&both)));
        assert!(!fields_widened(None, Some(&display)));
        assert!(!fields_widened(Some(&both), None));
    }

    #[test]
    fn profiles_settings_are_sticky() {
        let mut cached = Profiles::default();
        let mut request = Profiles::default();
        request.enabled = Some(true);

        assert!(!update_cache_profiles(&request, &mut cached));
        assert_eq!(cached.enabled, Some(true));

        assert!(!update_cache_profiles(&Profiles::default(), &mut cached));
        assert_eq!(cached.enabled, Some(true));
    }

    #[test]
    fn own_profile_is_owed_on_a_new_connection_only_when_enabled() {
        let mut conn = Connection::default();

        assert!(!conn.own_profile_owed());

        conn.extensions.profiles.enabled = Some(true);
        assert!(conn.own_profile_owed());

        conn.globalsince = 5;
        assert!(!conn.own_profile_owed());

        conn.own_profile_owed = true;
        assert!(conn.own_profile_owed());
    }
}
