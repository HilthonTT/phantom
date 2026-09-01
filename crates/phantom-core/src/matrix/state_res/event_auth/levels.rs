use super::*;

/// Is the user allowed to send a specific event based on the rooms power
/// levels.
///
/// Does the event have the correct userId as its state_key if it's not the ""
/// state_key.
pub(super) fn can_send_event(event: impl Event, ple: Option<impl Event>, user_level: Int) -> bool {
    let event_type_power_level = get_send_level(event.event_type(), event.state_key(), ple);

    debug!(
        required_level = i64::from(event_type_power_level),
        user_level = i64::from(user_level),
        state_key = ?event.state_key(),
        "permissions factors",
    );

    if user_level < event_type_power_level {
        return false;
    }

    if event.state_key().is_some_and(|k| k.starts_with('@'))
        && event.state_key() != Some(event.sender().as_str())
    {
        return false;
    }

    true
}

/// Confirm that the event sender has the required power levels.
pub(super) fn check_power_levels(
    room_version: &RoomVersion,
    power_event: impl Event,
    previous_power_event: Option<impl Event>,
    user_level: Int,
) -> Option<bool> {
    match power_event.state_key() {
        Some("") => {}
        Some(key) => {
            error!(
                state_key = key,
                "m.room.power_levels event has non-empty state key"
            );
            return None;
        }
        None => {
            error!("check_power_levels requires an m.room.power_levels *state* event argument");
            return None;
        }
    }

    let user_content: RoomPowerLevelsEventContent =
        deserialize_power_levels(power_event.content().get(), room_version)?;

    debug!("validation of power event finished");

    #[allow(clippy::manual_let_else)]
    let current_state = match previous_power_event {
        Some(current_state) => current_state,
        None => return Some(true),
    };

    let current_content: RoomPowerLevelsEventContent =
        deserialize_power_levels(current_state.content().get(), room_version)?;

    let mut user_levels_to_check = BTreeSet::new();
    let old_list = &current_content.users;
    let user_list = &user_content.users;
    for user in old_list.keys().chain(user_list.keys()) {
        let user: &UserId = user;
        user_levels_to_check.insert(user);
    }

    trace!(set = ?user_levels_to_check, "user levels to check");

    let mut event_levels_to_check = BTreeSet::new();
    let old_list = &current_content.events;
    let new_list = &user_content.events;
    for ev_id in old_list.keys().chain(new_list.keys()) {
        event_levels_to_check.insert(ev_id);
    }

    trace!(set = ?event_levels_to_check, "event levels to check");

    let old_state = &current_content;
    let new_state = &user_content;

    for user in user_levels_to_check {
        let old_level = old_state.users.get(user);
        let new_level = new_state.users.get(user);
        if old_level.is_some() && new_level.is_some() && old_level == new_level {
            continue;
        }

        if user != power_event.sender() && old_level == Some(&user_level) {
            warn!("m.room.power_level cannot remove ops == to own");
            return Some(false);
        }

        let old_level_too_big = old_level > Some(&user_level);
        let new_level_too_big = new_level > Some(&user_level);
        if old_level_too_big || new_level_too_big {
            warn!("m.room.power_level failed to add ops > than own");
            return Some(false);
        }
    }

    for ev_type in event_levels_to_check {
        let old_level = old_state.events.get(ev_type);
        let new_level = new_state.events.get(ev_type);
        if old_level.is_some() && new_level.is_some() && old_level == new_level {
            continue;
        }

        let old_level_too_big = old_level > Some(&user_level);
        let new_level_too_big = new_level > Some(&user_level);
        if old_level_too_big || new_level_too_big {
            warn!("m.room.power_level failed to add ops > than own");
            return Some(false);
        }
    }

    if room_version.limit_notifications_power_levels {
        let old_level = old_state.notifications.room;
        let new_level = new_state.notifications.room;
        if old_level != new_level {
            let old_level_too_big = old_level > user_level;
            let new_level_too_big = new_level > user_level;
            if old_level_too_big || new_level_too_big {
                warn!("m.room.power_level failed to add ops > than own");
                return Some(false);
            }
        }
    }

    let levels = [
        "users_default",
        "events_default",
        "state_default",
        "ban",
        "redact",
        "kick",
        "invite",
    ];
    let old_state = serde_json::to_value(old_state).unwrap();
    let new_state = serde_json::to_value(new_state).unwrap();
    for lvl_name in &levels {
        if let Some((old_lvl, new_lvl)) = get_deserialize_levels(&old_state, &new_state, lvl_name) {
            let old_level_too_big = old_lvl > user_level;
            let new_level_too_big = new_lvl > user_level;

            if old_level_too_big || new_level_too_big {
                warn!("cannot add ops > than own");
                return Some(false);
            }
        }
    }

    Some(true)
}

fn get_deserialize_levels(
    old: &serde_json::Value,
    new: &serde_json::Value,
    name: &str,
) -> Option<(Int, Int)> {
    Some((
        serde_json::from_value(old.get(name)?.clone()).ok()?,
        serde_json::from_value(new.get(name)?.clone()).ok()?,
    ))
}

/// Does the event redacting come from a user with enough power to redact the
/// given event.
pub(super) fn check_redaction(
    _room_version: &RoomVersion,
    redaction_event: impl Event,
    user_level: Int,
    redact_level: Int,
) -> Result<bool> {
    if user_level >= redact_level {
        debug!("redaction allowed via power levels");
        return Ok(true);
    }

    if redaction_event.event_id().borrow().server_name()
        == redaction_event
            .redacts()
            .as_ref()
            .and_then(|&id| id.borrow().server_name())
    {
        debug!("redaction event allowed via room version 1 rules");
        return Ok(true);
    }

    Ok(false)
}

/// Helper function to fetch the power level needed to send an event of type
/// `e_type` based on the rooms "m.room.power_level" event.
fn get_send_level(
    e_type: &TimelineEventType,
    state_key: Option<&str>,
    power_lvl: Option<impl Event>,
) -> Int {
    power_lvl
        .and_then(|ple| {
            from_json_str::<RoomPowerLevelsEventContent>(ple.content().get())
                .map(|content| {
                    content.events.get(e_type).copied().unwrap_or_else(|| {
                        if state_key.is_some() {
                            content.state_default
                        } else {
                            content.events_default
                        }
                    })
                })
                .ok()
        })
        .unwrap_or_else(|| {
            if state_key.is_some() {
                int!(50)
            } else {
                int!(0)
            }
        })
}
