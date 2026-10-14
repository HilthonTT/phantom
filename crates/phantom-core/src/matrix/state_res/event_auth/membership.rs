use ruma::{CanonicalJsonObject, CanonicalJsonValue, SigningKeyAlgorithm, signatures};

use super::*;

#[allow(clippy::too_many_arguments)]
#[allow(clippy::cognitive_complexity)]
pub(super) fn valid_membership_change(
    room_version: &RoomVersion,
    target_user: &UserId,
    target_user_membership_event: Option<&impl Event>,
    sender: &UserId,
    sender_membership_event: Option<&impl Event>,
    current_event: impl Event,
    current_third_party_invite: Option<&impl Event>,
    power_levels_event: Option<&impl Event>,
    join_rules_event: Option<&impl Event>,
    user_for_join_auth: Option<&UserId>,
    user_for_join_auth_membership: &MembershipState,
    create_room: &impl Event,
    creators: &[OwnedUserId],
) -> Result<bool> {
    #[derive(Deserialize)]
    struct GetThirdPartyInvite {
        third_party_invite: Option<Raw<ThirdPartyInvite>>,
    }
    let content = current_event.content();

    let target_membership = from_json_str::<GetMembership>(content.get())?.membership;
    let third_party_invite =
        from_json_str::<GetThirdPartyInvite>(content.get())?.third_party_invite;

    let sender_membership = match &sender_membership_event {
        Some(pdu) => from_json_str::<GetMembership>(pdu.content().get())?.membership,
        None => MembershipState::Leave,
    };
    let sender_is_joined = sender_membership == MembershipState::Join;

    let target_user_current_membership = match &target_user_membership_event {
        Some(pdu) => from_json_str::<GetMembership>(pdu.content().get())?.membership,
        None => MembershipState::Leave,
    };

    let power_levels: RoomPowerLevelsEventContent = match &power_levels_event {
        Some(ev) => from_json_str(ev.content().get())?,
        None => from_json_str("{}")?,
    };

    let is_creator = |user: &UserId| creators.iter().any(|creator| creator == user);

    let sender_power = if is_creator(sender) {
        Some(UserPower::Infinite)
    } else {
        power_levels
            .users
            .get(sender)
            .or_else(|| sender_is_joined.then_some(&power_levels.users_default))
            .copied()
            .map(UserPower::Level)
    };

    let target_power = if is_creator(target_user) {
        Some(UserPower::Infinite)
    } else {
        power_levels
            .users
            .get(target_user)
            .or_else(|| {
                (target_membership == MembershipState::Join).then_some(&power_levels.users_default)
            })
            .copied()
            .map(UserPower::Level)
    };

    let mut join_rules = JoinRule::Invite;
    if let Some(jr) = &join_rules_event {
        join_rules = from_json_str::<RoomJoinRulesEventContent>(jr.content().get())?.join_rule;
    }

    let power_levels_event_id = power_levels_event.as_ref().map(Event::event_id);
    let sender_membership_event_id = sender_membership_event.as_ref().map(Event::event_id);
    let target_user_membership_event_id =
        target_user_membership_event.as_ref().map(Event::event_id);

    let user_for_join_auth_is_valid = if let Some(user_for_join_auth) = user_for_join_auth {
        let (auth_user_pl, invite_level) = if let Some(pl) = &power_levels_event {
            let invite =
                deserialize_power_levels_content_invite(pl.content().get(), room_version)?.invite;

            let content =
                deserialize_power_levels_content_fields(pl.content().get(), room_version)?;
            let user_pl = match content.get_user_power(user_for_join_auth) {
                _ if is_creator(user_for_join_auth) => UserPower::Infinite,
                Some(level) => UserPower::Level(*level),
                _ => UserPower::Level(content.users_default),
            };

            (user_pl, UserPower::Level(invite))
        } else if is_creator(user_for_join_auth) {
            (UserPower::Infinite, UserPower::Level(int!(0)))
        } else {
            (UserPower::Level(int!(0)), UserPower::Level(int!(0)))
        };
        (user_for_join_auth_membership == &MembershipState::Join) && (auth_user_pl >= invite_level)
    } else {
        false
    };

    Ok(match target_membership {
        MembershipState::Join => {
            let mut prev_events = current_event.prev_events();

            let prev_event_is_create_event = prev_events
                .next()
                .is_some_and(|event_id| event_id.borrow() == create_room.event_id().borrow());
            let no_more_prev_events = prev_events.next().is_none();

            if prev_event_is_create_event && no_more_prev_events {
                let is_creator = if room_version.use_room_create_sender {
                    let creator = create_room.sender();

                    creator == sender && creator == target_user
                } else {
                    #[allow(deprecated)]
                    let creator =
                        from_json_str::<RoomCreateEventContent>(create_room.content().get())?
                            .creator
                            .ok_or_else(|| serde_json::Error::missing_field("creator"))?;

                    creator == sender && creator == target_user
                };

                if is_creator {
                    return Ok(true);
                }
            }

            if sender != target_user {
                warn!("Can't make other user join");
                false
            } else if target_user_current_membership == MembershipState::Ban {
                warn!(?target_user_membership_event_id, "Banned user can't join");
                false
            } else if (join_rules == JoinRule::Invite
                || room_version.allow_knocking && join_rules == JoinRule::Knock)
                && (target_user_current_membership == MembershipState::Join
                    || target_user_current_membership == MembershipState::Invite)
            {
                true
            } else if room_version.restricted_join_rules
                && matches!(join_rules, JoinRule::Restricted(_))
                || room_version.knock_restricted_join_rule
                    && matches!(join_rules, JoinRule::KnockRestricted(_))
            {
                if matches!(
                    target_user_current_membership,
                    MembershipState::Invite | MembershipState::Join
                ) {
                    true
                } else {
                    user_for_join_auth_is_valid
                }
            } else {
                join_rules == JoinRule::Public
            }
        }
        MembershipState::Invite => match third_party_invite.and_then(|i| i.deserialize().ok()) {
            Some(tp_id) => {
                if target_user_current_membership == MembershipState::Ban {
                    warn!(?target_user_membership_event_id, "Can't invite banned user");
                    false
                } else {
                    let allow = verify_third_party_invite(
                        Some(target_user),
                        sender,
                        &tp_id,
                        current_third_party_invite,
                    );
                    if !allow {
                        warn!("Third party invite invalid");
                    }
                    allow
                }
            }
            _ => {
                if !sender_is_joined
                    || target_user_current_membership == MembershipState::Join
                    || target_user_current_membership == MembershipState::Ban
                {
                    warn!(
                        ?target_user_membership_event_id,
                        ?sender_membership_event_id,
                        "Can't invite user if sender not joined or the user is currently \
							 joined or banned",
                    );
                    false
                } else {
                    let allow = sender_power
                        .filter(|&p| p >= UserPower::Level(power_levels.invite))
                        .is_some();
                    if !allow {
                        warn!(
                            ?target_user_membership_event_id,
                            ?power_levels_event_id,
                            "User does not have enough power to invite",
                        );
                    }
                    allow
                }
            }
        },
        MembershipState::Leave => {
            if sender == target_user {
                let allow = target_user_current_membership == MembershipState::Join
                    || target_user_current_membership == MembershipState::Invite
                    || target_user_current_membership == MembershipState::Knock;
                if !allow {
                    warn!(
                        ?target_user_membership_event_id,
                        ?target_user_current_membership,
                        "Can't leave if sender is not already invited, knocked, or joined"
                    );
                }
                allow
            } else if !sender_is_joined
                || target_user_current_membership == MembershipState::Ban
                    && sender_power
                        .filter(|&p| p < UserPower::Level(power_levels.ban))
                        .is_some()
            {
                warn!(
                    ?target_user_membership_event_id,
                    ?sender_membership_event_id,
                    "Can't kick if sender not joined or user is already banned",
                );
                false
            } else {
                let allow = sender_power
                    .filter(|&p| p >= UserPower::Level(power_levels.kick))
                    .is_some()
                    && target_power < sender_power;
                if !allow {
                    warn!(
                        ?target_user_membership_event_id,
                        ?power_levels_event_id,
                        "User does not have enough power to kick",
                    );
                }
                allow
            }
        }
        MembershipState::Ban => {
            if !sender_is_joined {
                warn!(
                    ?sender_membership_event_id,
                    "Can't ban user if sender is not joined"
                );
                false
            } else {
                let allow = sender_power
                    .filter(|&p| p >= UserPower::Level(power_levels.ban))
                    .is_some()
                    && target_power < sender_power;
                if !allow {
                    warn!(
                        ?target_user_membership_event_id,
                        ?power_levels_event_id,
                        "User does not have enough power to ban",
                    );
                }
                allow
            }
        }
        MembershipState::Knock if room_version.allow_knocking => {
            if !matches!(join_rules, JoinRule::KnockRestricted(_) | JoinRule::Knock) {
                warn!("Join rule is not set to knock or knock_restricted, knocking is not allowed");
                false
            } else if matches!(join_rules, JoinRule::KnockRestricted(_))
                && !room_version.knock_restricted_join_rule
            {
                warn!(
                    "Join rule is set to knock_restricted but room version does not support \
					 knock_restricted, knocking is not allowed"
                );
                false
            } else if sender != target_user {
                warn!(
                    ?sender,
                    ?target_user,
                    "Can't make another user knock, sender did not match target"
                );
                false
            } else if matches!(
                sender_membership,
                MembershipState::Ban | MembershipState::Invite | MembershipState::Join
            ) {
                warn!(
                    ?target_user_membership_event_id,
                    "Knocking with a membership state of ban, invite or join is invalid",
                );
                false
            } else {
                true
            }
        }
        _ => {
            warn!("Unknown membership transition");
            false
        }
    })
}

/// The third-party invite rules of `m.room.member`: the invite's `signed`
/// block names the target, answers an `m.room.third_party_invite` the same
/// sender put in the room, and carries a signature by one of that event's
/// public keys.
///
/// ruma's state resolution, which this was ported from, compared the token
/// itself against the public keys instead of verifying the signature, so no
/// genuine third-party invite could ever pass; this checks the signature as
/// the spec's rule 4.4.7 asks.
pub(super) fn verify_third_party_invite(
    target_user: Option<&UserId>,
    sender: &UserId,
    tp_id: &ThirdPartyInvite,
    current_third_party_invite: Option<impl Event>,
) -> bool {
    let Ok(signed) = tp_id.signed.deserialize() else {
        return false;
    };

    if target_user != Some(&signed.mxid) {
        return false;
    }

    let Some(current_tpid) = current_third_party_invite else {
        return false;
    };

    if current_tpid.state_key() != Some(signed.token.as_str()) {
        return false;
    }

    if sender != current_tpid.sender() {
        return false;
    }

    let Ok(tpid_ev) =
        from_json_str::<RoomThirdPartyInviteEventContent>(current_tpid.content().get())
    else {
        return false;
    };

    let public_keys: Vec<Vec<u8>> = tpid_ev
        .public_keys
        .unwrap_or_default()
        .iter()
        .map(|key| &key.public_key)
        .chain(std::iter::once(&tpid_ev.public_key))
        .filter_map(|key| key.decode().ok())
        .collect();

    signed_by_any(tp_id.signed.json().get(), &public_keys)
}

/// Whether any ed25519 signature on the `signed` JSON object verifies under
/// any of `public_keys`, whoever the signature is attributed to.
fn signed_by_any(signed: &str, public_keys: &[Vec<u8>]) -> bool {
    let Ok(object) = from_json_str::<CanonicalJsonObject>(signed) else {
        return false;
    };

    let Ok(canonical) = signatures::to_canonical_json_string_for_signing(&object) else {
        return false;
    };

    let Some(CanonicalJsonValue::Object(by_entity)) = object.get("signatures") else {
        return false;
    };

    by_entity
        .values()
        .filter_map(|keys| match keys {
            CanonicalJsonValue::Object(keys) => Some(keys),
            _ => None,
        })
        .flatten()
        .filter(|(key_id, _)| key_id.starts_with("ed25519:"))
        .filter_map(|(_, signature)| match signature {
            CanonicalJsonValue::String(signature) => Base64::<Standard>::parse(signature).ok(),
            _ => None,
        })
        .any(|signature| {
            public_keys.iter().any(|key| {
                signatures::verify_canonical_json_bytes(
                    &SigningKeyAlgorithm::Ed25519,
                    key,
                    signature.as_bytes(),
                    canonical.as_bytes(),
                )
                .is_ok()
            })
        })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use ruma::{
        CanonicalJsonObject,
        events::room::member::ThirdPartyInvite,
        serde::Raw,
        signatures::{Ed25519KeyPair, sign_json},
    };
    use serde_json::json;

    use super::verify_third_party_invite;
    use crate::matrix::state_res::{
        TimelineEventType,
        fixtures::{alice, bob, charlie, to_pdu_event},
    };

    const TOKEN: &str = "abc123";

    fn key_pair() -> Ed25519KeyPair {
        let document = Ed25519KeyPair::generate();

        Ed25519KeyPair::from_der(&document, "0".to_owned()).expect("a parseable key")
    }

    fn raw<T: serde::Serialize, U>(value: &T) -> Raw<U> {
        Raw::from_json(serde_json::value::to_raw_value(value).unwrap())
    }

    /// The invite's `signed` block, for `mxid`, signed by `key`.
    fn invite(key: &Ed25519KeyPair, mxid: &str) -> ThirdPartyInvite {
        let mut signed: CanonicalJsonObject =
            serde_json::from_value(json!({ "mxid": mxid, "token": TOKEN })).unwrap();
        sign_json("identity.test", key, &mut signed).unwrap();

        ThirdPartyInvite::new("a...@example.com".to_owned(), raw(&signed))
    }

    /// The room's `m.room.third_party_invite` from `sender`, naming `key`.
    fn room_invite(
        key: &Ed25519KeyPair,
        sender: &ruma::UserId,
    ) -> std::sync::Arc<crate::matrix::state_res::fixtures::PduEvent> {
        let public_key =
            ruma::serde::Base64::<ruma::serde::base64::Standard, _>::new(key.public_key()).encode();
        let content = json!({
            "display_name": "a...@example.com",
            "key_validity_url": "https://identity.test/_matrix/identity/v2/pubkey/isvalid",
            "public_key": public_key,
        });

        to_pdu_event(
            "TPI",
            sender,
            TimelineEventType::RoomThirdPartyInvite,
            Some(TOKEN),
            serde_json::value::to_raw_value(&content).unwrap(),
            &["CREATE"],
            &["CREATE"],
        )
    }

    #[test]
    fn a_signed_invite_from_the_same_sender_passes() {
        let key = key_pair();
        let tpi = room_invite(&key, alice());

        assert!(verify_third_party_invite(
            Some(bob()),
            alice(),
            &invite(&key, bob().as_str()),
            Some(&*tpi),
        ));
    }

    #[test]
    fn a_signature_by_another_key_fails() {
        let tpi = room_invite(&key_pair(), alice());

        assert!(!verify_third_party_invite(
            Some(bob()),
            alice(),
            &invite(&key_pair(), bob().as_str()),
            Some(&*tpi),
        ));
    }

    #[test]
    fn the_wrong_target_or_sender_fails() {
        let key = key_pair();
        let tpi = room_invite(&key, alice());
        let signed = invite(&key, bob().as_str());

        assert!(!verify_third_party_invite(
            Some(charlie()),
            alice(),
            &signed,
            Some(&*tpi)
        ));
        assert!(!verify_third_party_invite(
            Some(bob()),
            charlie(),
            &signed,
            Some(&*tpi)
        ));
    }

    #[test]
    fn a_tampered_block_fails() {
        let key = key_pair();
        let tpi = room_invite(&key, alice());
        let genuine = invite(&key, bob().as_str());

        let mut tampered: BTreeMap<String, serde_json::Value> =
            serde_json::from_str(genuine.signed.json().get()).unwrap();
        tampered.insert("extra".to_owned(), json!(true));
        let tampered = ThirdPartyInvite::new(genuine.display_name.clone(), raw(&tampered));

        assert!(!verify_third_party_invite(
            Some(bob()),
            alice(),
            &tampered,
            Some(&*tpi)
        ));
    }

    #[test]
    fn no_room_invite_fails() {
        let key = key_pair();

        assert!(!verify_third_party_invite(
            Some(bob()),
            alice(),
            &invite(&key, bob().as_str()),
            None::<&crate::matrix::state_res::fixtures::PduEvent>,
        ));
    }
}
