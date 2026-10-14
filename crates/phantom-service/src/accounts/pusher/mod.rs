//! Push gateways: the pushers a client registers, the notifications sent to
//! them, and the record of notified events `GET /notifications` lists.
//!
//! An event reaches a pusher in two halves. `rooms::timeline`'s append path
//! evaluates each recipient's push rules once, counts and records the event
//! for those it notifies, and queues it on the sending service for each of
//! their pushers. The sending service then calls [`Service::send_push_notice`],
//! which evaluates the rules again (they may have changed while the event was
//! queued) and posts to the gateway.

mod notified;
#[cfg(test)]
mod tests;

use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use futures::{Stream, StreamExt};
use ipaddress::IPAddress;
use phantom_core::{
    Err, Error, Result, debug, debug_info, err, http, implement,
    matrix::pdu::{PduCount, PduEvent, PduId, RawPduId},
    runtime::server::Server,
    stream::TryIgnore,
    text::string_from_bytes,
    time::now_millis,
    trace, warn,
};
use phantom_database::{Deserialized, Ignore, Interfix, Json, Map, table};
use ruma::{
    DeviceId, OwnedDeviceId, RoomId, UInt, UserId,
    api::{
        client::push::{Pusher, PusherKind, set_pusher},
        error::ErrorKind,
        push_gateway::send_event_notification::v1::{Device, Notification, NotificationPriority},
    },
    events::{
        AnySyncTimelineEvent, GlobalAccountDataEventType, TimelineEventType,
        push_rules::PushRulesEvent, room::power_levels::RoomPowerLevels,
    },
    push::{
        Action, HighlightTweakValue, HttpPusherData, PushConditionRoomCtx, PushFormat, Ruleset,
        Tweak,
    },
    serde::Raw,
    uint,
};
use serde::Deserialize;

pub use self::notified::Notified;
use crate::{
    Dep,
    accounts::{account_data, profile},
    net::client,
    rooms,
};

pub struct Service {
    db: Data,
    services: Services,
}

struct Services {
    server: Arc<Server>,
    account_data: Dep<account_data::Service>,
    client: Dep<client::Service>,
    state_accessor: Dep<rooms::state_accessor::Service>,
    state_cache: Dep<rooms::state_cache::Service>,
    profile: Dep<profile::Service>,
    user: Dep<rooms::user::Service>,
}

struct Data {
    senderkey_pusher: Arc<Map>,
    pushkey_deviceid: Arc<Map>,
    useridcount_notification: Arc<Map>,
}

/// What became of one event handed to a pusher.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Delivery {
    /// The gateway accepted the notification.
    Sent,

    /// The user's rules do not notify for the event, or the pusher is not one
    /// phantom delivers to, so nothing was sent.
    Skipped,

    /// The gateway named the pushkey as rejected and the pusher was removed;
    /// nothing more should be sent through it.
    Removed,
}

const PUSHKEY_MAX_LEN: usize = 512;

const APP_ID_MAX_LEN: usize = 64;

/// How often old rows of the notified-event list are swept.
const SWEEP_INTERVAL: Duration = Duration::from_secs(60 * 60);

const DAY_MS: u64 = 24 * 60 * 60 * 1000;

#[async_trait]
impl crate::Service for Service {
    fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
        Ok(Arc::new(Self {
            db: Data {
                senderkey_pusher: args.db[table::SENDERKEY_PUSHER].clone(),
                pushkey_deviceid: args.db[table::PUSHKEY_DEVICEID].clone(),
                useridcount_notification: args.db[table::USERIDCOUNT_NOTIFICATION].clone(),
            },
            services: Services {
                server: args.server.clone(),
                account_data: args.depend::<account_data::Service>(),
                client: args.depend::<client::Service>(),
                state_accessor: args.depend::<rooms::state_accessor::Service>(),
                state_cache: args.depend::<rooms::state_cache::Service>(),
                profile: args.depend::<profile::Service>(),
                user: args.depend::<rooms::user::Service>(),
            },
        }))
    }

    async fn worker(self: Arc<Self>) -> Result {
        loop {
            let config = &self.services.server.config.client;
            let max_age_ms = config.notification_retention_days.saturating_mul(DAY_MS);
            let max_per_user = config.notification_max_per_user;

            let removed = notified::prune(
                &self.db.useridcount_notification,
                now_millis(),
                max_age_ms,
                max_per_user,
            )
            .await;

            if removed > 0 {
                debug_info!(removed, "Pruned old notified events");
            }

            tokio::select! {
                () = tokio::time::sleep(SWEEP_INTERVAL) => {},
                () = self.services.server.until_shutdown() => return Ok(()),
            }
        }
    }

    fn name(&self) -> &str {
        crate::make_name(std::module_path!())
    }
}

#[implement(Service)]
pub async fn set_pusher(
    &self,
    sender: &UserId,
    sender_device: &DeviceId,
    pusher: &set_pusher::v3::PusherAction,
) -> Result {
    match pusher {
        set_pusher::v3::PusherAction::Post(data) => {
            let pushkey = data.pusher.ids.pushkey.as_str();

            if pushkey.len() > PUSHKEY_MAX_LEN {
                return Err!(Request(InvalidParam(
                    "Push key length cannot be greater than {PUSHKEY_MAX_LEN} bytes."
                )));
            }

            if data.pusher.ids.app_id.as_str().len() > APP_ID_MAX_LEN {
                return Err!(Request(InvalidParam(
                    "App ID length cannot be greater than {APP_ID_MAX_LEN} bytes."
                )));
            }

            if let PusherKind::Http(http) = &data.pusher.kind {
                self.check_gateway_url(&http.url)?;
            }

            // Without `append`, a pushkey belongs to one user: a device that
            // changed hands must stop receiving the previous owner's pushes.
            if !data.append {
                self.delete_pusher_for_others(sender, &data.pusher).await?;
            }

            self.db
                .senderkey_pusher
                .put((sender, pushkey), Json(pusher))?;
            self.db.pushkey_deviceid.insert(pushkey, sender_device)?;
        }
        set_pusher::v3::PusherAction::Delete(ids) => {
            self.delete_pusher(sender, ids.pushkey.as_str())?;
        }
        _ => return Err!(Request(InvalidParam("Unrecognised pusher action."))),
    }

    Ok(())
}

/// Removes every other user's pusher with the same app id and pushkey.
///
/// Pushers are keyed by user, so this walks the table; it runs only when a
/// client registers a pusher, and the table holds one row per device.
#[implement(Service)]
async fn delete_pusher_for_others(&self, sender: &UserId, pusher: &Pusher) -> Result {
    let others: Vec<(String, String)> = self
        .db
        .senderkey_pusher
        .stream()
        .ignore_err()
        .filter_map(|((user, pushkey), existing): ((&str, &str), Pusher)| {
            let other = user != sender.as_str()
                && pushkey == pusher.ids.pushkey
                && existing.ids.app_id == pusher.ids.app_id;

            futures::future::ready(other.then(|| (user.to_owned(), pushkey.to_owned())))
        })
        .boxed()
        .collect()
        .await;

    for (user, pushkey) in others {
        debug!(%user, %pushkey, "Removing pusher taken over by another user");
        self.db
            .senderkey_pusher
            .del((user.as_str(), pushkey.as_str()))?;
    }

    Ok(())
}

#[implement(Service)]
pub fn delete_pusher(&self, sender: &UserId, pushkey: &str) -> Result {
    self.db.senderkey_pusher.del((sender, pushkey))?;
    self.db.pushkey_deviceid.remove(pushkey)?;

    Ok(())
}

#[implement(Service)]
pub async fn get_pusher_device(&self, pushkey: &str) -> Result<OwnedDeviceId> {
    self.db.pushkey_deviceid.get(pushkey).await.deserialized()
}

#[implement(Service)]
pub async fn get_pusher(&self, sender: &UserId, pushkey: &str) -> Result<Pusher> {
    self.db
        .senderkey_pusher
        .qry(&(sender, pushkey))
        .await
        .deserialized()
}

#[implement(Service)]
pub async fn get_pushers(&self, sender: &UserId) -> Vec<Pusher> {
    let prefix = (sender, Interfix);

    self.db
        .senderkey_pusher
        .stream_prefix(&prefix)
        .ignore_err()
        .map(|(_, pusher): (Ignore, Pusher)| pusher)
        .collect()
        .await
}

#[implement(Service)]
pub fn get_pushkeys<'a>(&'a self, sender: &'a UserId) -> impl Stream<Item = &'a str> + Send + 'a {
    let prefix = (sender, Interfix);

    self.db
        .senderkey_pusher
        .keys_prefix(&prefix)
        .ignore_err()
        .map(|(_, pushkey): (Ignore, &str)| pushkey)
}

/// The push rules a user's events are evaluated against: their own, or the
/// server default when they have never stored any.
#[implement(Service)]
pub async fn ruleset(&self, user_id: &UserId) -> Ruleset {
    self.services
        .account_data
        .get_global(user_id, GlobalAccountDataEventType::PushRules)
        .await
        .map_or_else(
            |_| Ruleset::server_default(user_id),
            |event: PushRulesEvent| event.content.global,
        )
}

/// Records that `pdu_id` notified `user_id`, for `GET /notifications`.
///
/// A backfilled event has no forward position to page by, and is never
/// appended live, so it is not recorded.
#[implement(Service)]
pub fn record_notification(
    &self,
    user_id: &UserId,
    pdu_id: &RawPduId,
    actions: &[Action],
) -> Result {
    let pdu_id = PduId::from(*pdu_id);
    let PduCount::Normal(count) = pdu_id.shorteventid else {
        return Ok(());
    };

    let notified = Notified {
        ts: now_millis(),
        sroomid: pdu_id.shortroomid,
        tag: None,
        actions: actions.to_vec(),
    };

    notified::put(&self.db.useridcount_notification, user_id, count, &notified)
}

/// One page of the user's notified events, newest first, and the token for
/// the page after it.
///
/// `from` is the token a previous page returned; rows below it are listed.
/// `None` for the token means there is nothing older.
#[implement(Service)]
pub async fn notifications_page(
    &self,
    user_id: &UserId,
    from: Option<u64>,
    limit: usize,
    only_highlight: bool,
) -> (Vec<(u64, Notified)>, Option<u64>) {
    notified::page(
        &self.db.useridcount_notification,
        user_id,
        from,
        limit,
        only_highlight,
    )
    .await
}

/// Removes every notified event recorded for the user.
#[implement(Service)]
pub async fn delete_notifications(&self, user_id: &UserId) {
    self.db
        .useridcount_notification
        .del_prefix(&(user_id, Interfix))
        .await;
}

#[implement(Service)]
fn check_gateway_url(&self, url: &str) -> Result<reqwest::Url> {
    let parsed = reqwest::Url::parse(url).map_err(|e| {
        err!(Request(InvalidParam(warn!(
            "Pusher URL {url:?} is not a URL: {e}"
        ))))
    })?;

    if !matches!(parsed.scheme(), "http" | "https") {
        return Err!(Request(InvalidParam(warn!(
            "Pusher URL {url:?} is not an HTTP or HTTPS URL"
        ))));
    }

    if let Some(ip) = host_ip_literal(&parsed)
        && !self.services.client.valid_cidr_range(&ip)
    {
        return Err!(Request(InvalidParam(warn!(
            "Pusher URL {url:?} is a forbidden remote address"
        ))));
    }

    Ok(parsed)
}

fn host_ip_literal(url: &reqwest::Url) -> Option<IPAddress> {
    let host = url.host_str()?;
    IPAddress::parse(host.trim_start_matches('[').trim_end_matches(']')).ok()
}

/// Whether a failed delivery is worth retrying.
///
/// A gateway that answered 4xx (other than 429) refused this request and will
/// refuse it again, and a pusher URL phantom will not post to does not become
/// acceptable later; everything else (timeouts, connection failures, 5xx) may
/// clear up, so the sending service keeps the event and backs off.
#[must_use]
pub fn is_permanent_failure(error: &Error) -> bool {
    let status = error.status_code();

    status.is_client_error() && status != http::StatusCode::TOO_MANY_REQUESTS
}

#[derive(Deserialize)]
struct GatewayResponse {
    #[serde(default)]
    rejected: Vec<String>,
}

/// Posts `{"notification": ...}` to a push gateway and returns the pushkeys it
/// rejected.
///
/// The body is built as JSON rather than through ruma's request type because
/// ruma drops `counts` when the unread count is zero, and a zero is exactly
/// what clears a badge after the user reads everything. The URL is used as
/// registered: the spec requires it to be the gateway's full
/// `/_matrix/push/v1/notify` URL.
#[implement(Service)]
#[tracing::instrument(skip(self, notification), level = "debug")]
async fn post_notification(
    &self,
    url: &str,
    mut notification: serde_json::Value,
    unread: Option<UInt>,
) -> Result<Vec<String>> {
    let parsed = self.check_gateway_url(url)?;

    if let (Some(unread), Some(object)) = (unread, notification.as_object_mut()) {
        object.insert("counts".to_owned(), serde_json::json!({ "unread": unread }));
    }

    let body = serde_json::json!({ "notification": notification });

    trace!(%url, ?body, "Posting to push gateway");

    let response = self
        .services
        .client
        .pusher
        .post(parsed)
        .header(http::header::CONTENT_TYPE, "application/json")
        .body(serde_json::to_vec(&body)?)
        .send()
        .await
        .inspect_err(|e| warn!("Could not send request to pusher {url}: {e}"))?;

    if let Some(remote_addr) = response.remote_addr()
        && let Ok(ip) = IPAddress::parse(remote_addr.ip().to_string())
        && !self.services.client.valid_cidr_range(&ip)
    {
        return Err!(Request(Forbidden(
            "Not allowed to send requests to this IP"
        )));
    }

    let status = response.status();
    let bytes = response.bytes().await?;

    if !status.is_success() {
        let message = format!(
            "Push gateway {url} returned unsuccessful HTTP response {status}: {:?}",
            string_from_bytes(&bytes)
        );

        warn!("{message}");

        // Keep the gateway's status so the sender can tell a refusal from an
        // outage; see `is_permanent_failure`.
        return Err(if status.is_client_error() {
            Error::Request(ErrorKind::Unknown, message.into(), status)
        } else {
            err!(BadServerResponse("{message}"))
        });
    }

    // The gateway took the notification; a body it got wrong only costs the
    // list of rejected pushkeys, and failing here would resend what it has.
    let rejected = serde_json::from_slice::<GatewayResponse>(&bytes)
        .map(|response| response.rejected)
        .unwrap_or_else(|e| {
            debug!("Push gateway {url} returned an unreadable response: {e}");
            Vec::new()
        });

    Ok(rejected)
}

/// Posts a notification for `pdu` through `pusher` if the user's rules
/// notify for it.
#[implement(Service)]
#[tracing::instrument(skip(self, pusher, ruleset, pdu), level = "debug")]
pub async fn send_push_notice(
    &self,
    user: &UserId,
    pusher: &Pusher,
    ruleset: &Ruleset,
    pdu: &PduEvent,
) -> Result<Delivery> {
    let power_levels = self
        .services
        .state_accessor
        .room_power_levels(&pdu.room_id)
        .await;

    let actions = self
        .get_actions(
            user,
            ruleset,
            power_levels,
            &pdu.to_sync_room_event(),
            &pdu.room_id,
        )
        .await;

    if !actions.iter().any(Action::should_notify) {
        return Ok(Delivery::Skipped);
    }

    let tweaks = actions
        .iter()
        .filter_map(|action| match action {
            Action::SetTweak(tweak) => Some(tweak.clone()),
            _ => None,
        })
        .collect();

    self.send_notice(user, pusher, tweaks, pdu).await
}

#[implement(Service)]
#[tracing::instrument(skip(self, user, ruleset, pdu), level = "debug")]
pub async fn get_actions<'a>(
    &self,
    user: &UserId,
    ruleset: &'a Ruleset,
    power_levels: RoomPowerLevels,
    pdu: &Raw<AnySyncTimelineEvent>,
    room_id: &RoomId,
) -> &'a [Action] {
    let member_count = self
        .services
        .state_cache
        .room_joined_count(room_id)
        .await
        .unwrap_or(1)
        .try_into()
        .unwrap_or_else(|_| uint!(0));

    let user_display_name = self
        .services
        .profile
        .displayname(user)
        .await
        .unwrap_or_else(|_| user.localpart().to_owned());

    let mut ctx = PushConditionRoomCtx::new(
        room_id.to_owned(),
        member_count,
        user.to_owned(),
        user_display_name,
    );

    ctx.power_levels = Some(power_levels.into());

    ruleset.get_actions(pdu, &ctx).await
}

/// The badge to send through `http`, or `None` when the client asked for no
/// badge (MSC4076).
#[implement(Service)]
async fn badge(&self, user: &UserId, http: &HttpPusherData) -> Option<UInt> {
    if badge_count_disabled(http) {
        return None;
    }

    let unread = self.services.user.global_notification_count(user).await;

    Some(UInt::new(unread).unwrap_or(UInt::MAX))
}

fn badge_count_disabled(http: &HttpPusherData) -> bool {
    [
        "org.matrix.msc4076.disable_badge_count",
        "disable_badge_count",
    ]
    .iter()
    .any(|key| http.data.get(*key).and_then(serde_json::Value::as_bool) == Some(true))
}

fn gateway_device(pusher: &Pusher, http: &HttpPusherData) -> Device {
    let mut device = Device::new(pusher.ids.app_id.clone(), pusher.ids.pushkey.clone());
    device.data.data.clone_from(&http.data);
    device.data.format.clone_from(&http.format);

    device
}

#[implement(Service)]
#[tracing::instrument(skip(self, pusher, tweaks, event), level = "debug")]
async fn send_notice(
    &self,
    user: &UserId,
    pusher: &Pusher,
    tweaks: Vec<Tweak>,
    event: &PduEvent,
) -> Result<Delivery> {
    // Email pushers are accepted and listed, but phantom sends no mail for
    // them; see docs/architecture.md.
    let PusherKind::Http(http) = &pusher.kind else {
        return Ok(Delivery::Skipped);
    };

    let event_id_only = http.format == Some(PushFormat::EventIdOnly);

    let mut device = gateway_device(pusher, http);

    if !event_id_only {
        device.tweaks.clone_from(&tweaks);
    }

    let mut notifi = Notification::new(vec![device]);
    notifi.event_id = Some((*event.event_id).to_owned());
    notifi.room_id = Some((*event.room_id).to_owned());

    if !event_id_only {
        notifi.prio = if event.kind == TimelineEventType::RoomEncrypted
            || tweaks.iter().any(|t| {
                matches!(
                    t,
                    Tweak::Highlight(HighlightTweakValue::Yes) | Tweak::Sound(_)
                )
            }) {
            NotificationPriority::High
        } else {
            NotificationPriority::Low
        };

        notifi.sender = Some(event.sender.clone());
        notifi.event_type = Some(event.kind.clone());
        notifi.content = serde_json::value::to_raw_value(&event.content).ok();

        if event.kind == TimelineEventType::RoomMember {
            notifi.user_is_target = event.state_key.as_deref() == Some(user.as_str());
        }

        notifi.sender_display_name = self.services.profile.displayname(&event.sender).await.ok();
        notifi.room_name = self
            .services
            .state_accessor
            .get_name(&event.room_id)
            .await
            .ok();
        notifi.room_alias = self
            .services
            .state_accessor
            .get_canonical_alias(&event.room_id)
            .await
            .ok();
    }

    let unread = self.badge(user, http).await;

    self.deliver(user, pusher, http, &notifi, unread).await
}

/// Tells each of the user's gateways the user's current unread total, with no
/// event attached, so a badge falls when the user reads on another device.
///
/// A failure is logged and dropped: the next notification carries the count
/// again.
#[implement(Service)]
#[tracing::instrument(skip(self), level = "debug")]
pub async fn send_badge_notices(&self, user: &UserId) {
    for pusher in self.get_pushers(user).await {
        let PusherKind::Http(http) = &pusher.kind else {
            continue;
        };

        let Some(unread) = self.badge(user, http).await else {
            continue;
        };

        let notifi = Notification::new(vec![gateway_device(&pusher, http)]);

        if let Err(e) = self
            .deliver(user, &pusher, http, &notifi, Some(unread))
            .await
        {
            debug!(%user, pushkey = %pusher.ids.pushkey, "Badge update not delivered: {e}");
        }
    }
}

/// Posts `notifi` and acts on the gateway's verdict about the pushkey.
#[implement(Service)]
async fn deliver(
    &self,
    user: &UserId,
    pusher: &Pusher,
    http: &HttpPusherData,
    notifi: &Notification,
    unread: Option<UInt>,
) -> Result<Delivery> {
    let notification = serde_json::to_value(notifi)?;

    let rejected = self
        .post_notification(&http.url, notification, unread)
        .await?;

    let pushkey = &pusher.ids.pushkey;
    if rejected.iter().any(|key| key == pushkey) {
        warn!(
            %user,
            %pushkey,
            url = %http.url,
            "Push gateway rejected the pushkey; removing the pusher",
        );

        self.delete_pusher(user, pushkey)?;

        return Ok(Delivery::Removed);
    }

    Ok(Delivery::Sent)
}

/// Runs [`Service::send_badge_notices`] in the background, so the request
/// that cleared the counts does not wait on the user's gateways.
#[implement(Service)]
pub fn refresh_badges(self: &Arc<Self>, user: &UserId) {
    let this = Arc::clone(self);
    let user = user.to_owned();

    self.services.server.runtime().spawn(async move {
        this.send_badge_notices(&user).await;
    });
}
