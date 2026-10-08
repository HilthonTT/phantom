use phantom_core::{Result, debug_warn, err, warn};
use phantom_service::Services;
use ruma::{
    RoomId, UserId,
    api::client::room::create_room::v3::CreationContent,
    events::{GlobalAccountDataEventType, push_rules::PushRulesEvent, room::create::PreviousRoom},
    push::{AnyPushRuleRef, NewPushRule, NewSimplePushRule, RuleKind},
    serde::Raw,
};

pub(super) async fn copy_creator_predecessor_push_rule(
    services: &Services,
    creation_content: Option<&Raw<CreationContent>>,
    sender_user: &UserId,
    room_id: &RoomId,
) {
    let Some(from_room) = creation_content
        .and_then(|content| {
            content
                .get_field::<PreviousRoom>("predecessor")
                .ok()
                .flatten()
        })
        .map(|predecessor| predecessor.room_id)
    else {
        return;
    };

    copy_room_push_rule(services, sender_user, &from_room, room_id)
        .await
        .inspect_err(|e| warn!(%e, "Failed to copy predecessor push rules"))
        .ok();
}

/// Copies the user's room-specific push rule for `from_room`, if any, to
/// `to_room`, keeping its actions and enabled flag.
pub(in super::super) async fn copy_room_push_rule(
    services: &Services,
    user_id: &UserId,
    from_room: &RoomId,
    to_room: &RoomId,
) -> Result {
    let Ok(mut account_data): Result<PushRulesEvent> = services
        .account_data
        .get_global(user_id, GlobalAccountDataEventType::PushRules)
        .await
    else {
        return Ok(());
    };

    let ruleset = &mut account_data.content.global;

    let Some(AnyPushRuleRef::Room(rule)) = ruleset.get(RuleKind::Room, from_room) else {
        return Ok(());
    };

    let actions = rule.actions.clone();
    let enabled = rule.enabled;

    let rule = NewPushRule::Room(NewSimplePushRule::new(to_room.to_owned(), actions));
    if let Err(e) = ruleset.insert(rule, None, None) {
        debug_warn!(%user_id, %to_room, "Could not copy the push rule: {e}");
        return Ok(());
    }

    ruleset
        .set_enabled(RuleKind::Room, to_room, enabled)
        .map_err(|e| err!(Database("Copied push rule went missing: {e}")))?;

    services
        .account_data
        .update(
            None,
            user_id,
            GlobalAccountDataEventType::PushRules.to_string().into(),
            &serde_json::to_value(account_data)?,
        )
        .await
}
