//! Push notifications, and the gateways they are sent through.
//!
//! A user's client registers a *pusher*: a gateway URL and a key identifying
//! the device to wake. When an event arrives, the user's push rules are
//! evaluated against it, and if they say to notify, a notification goes to
//! every pusher that user registered.
//!
//! The server never talks to Apple or Google. It talks to the gateway the
//! client named, which is what holds the platform credentials — so the URL is
//! attacker-supplied, and is checked against the CIDR denylist here and again
//! when it is used, since a name that resolved inside the network once may do
//! so again.
//!
//! Evaluating the rules is ruma's; what this service supplies is the context
//! they need — the room's power levels, its member count, the user's display
//! name — because a rule can say "notify me when someone says my name" or
//! "when someone who can ban me posts".

use std::{fmt::Debug, mem, sync::Arc};

use bytes::BytesMut;
use futures::{Stream, StreamExt};
use ipaddress::IPAddress;
use phantom_core::{
    Err, Result, err, http, implement, matrix::pdu::PduEvent, server::Server, stream::TryIgnore,
    text::string_from_bytes, trace, warn,
};
use phantom_database::{Deserialized, Ignore, Interfix, Json, Map};
use ruma::{
    DeviceId, OwnedDeviceId, RoomId, UInt, UserId,
    api::{
        IncomingResponse, Metadata, OutgoingRequest,
        auth_scheme::NoAuthentication,
        client::push::{Pusher, PusherKind, set_pusher},
        path_builder::SinglePath,
        push_gateway::send_event_notification::{
            self,
            v1::{Device, Notification, NotificationCounts, NotificationPriority},
        },
    },
    events::{
        AnySyncTimelineEvent, StateEventType, TimelineEventType,
        room::power_levels::{RoomPowerLevels, RoomPowerLevelsEventContent, RoomPowerLevelsSource},
    },
    push::{Action, HighlightTweakValue, PushConditionRoomCtx, PushFormat, Ruleset, Tweak},
    serde::Raw,
    uint,
};

use crate::{Dep, client, rooms, users};

pub struct Service {
    db: Data,
    services: Services,
}

struct Services {
    server: Arc<Server>,
    client: Dep<client::Service>,
    state_accessor: Dep<rooms::state_accessor::Service>,
    state_cache: Dep<rooms::state_cache::Service>,
    users: Dep<users::Service>,
}

struct Data {
    senderkey_pusher: Arc<Map>,
    pushkey_deviceid: Arc<Map>,
}

/// The longest push key a client may register. It identifies a device to the
/// gateway, and anything this long is not one.
const PUSHKEY_MAX_LEN: usize = 512;

/// The longest app id a client may register.
const APP_ID_MAX_LEN: usize = 64;

impl crate::Service for Service {
    fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
        Ok(Arc::new(Self {
            db: Data {
                senderkey_pusher: args.db["senderkey_pusher"].clone(),
                pushkey_deviceid: args.db["pushkey_deviceid"].clone(),
            },
            services: Services {
                server: args.server.clone(),
                client: args.depend::<client::Service>("client"),
                state_accessor: args
                    .depend::<rooms::state_accessor::Service>("rooms::state_accessor"),
                state_cache: args.depend::<rooms::state_cache::Service>("rooms::state_cache"),
                users: args.depend::<users::Service>("users"),
            },
        }))
    }

    fn name(&self) -> &str {
        crate::make_name(std::module_path!())
    }
}

/// Registers or removes a pusher for one of a user's devices.
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

/// Checks a gateway URL a client asked us to post to.
///
/// The scheme has to be one we can speak, and where the host is already an
/// address, it has to be one the denylist allows. A host that is a name is
/// checked again once it has resolved, in [`Service::send_request`].
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

    if let Some(host) = parsed.host_str()
        && let Ok(ip) = IPAddress::parse(host)
        && !self.services.client.valid_cidr_range(&ip)
    {
        return Err!(Request(InvalidParam(warn!(
            "Pusher URL {url:?} is a forbidden remote address"
        ))));
    }

    Ok(parsed)
}

/// Sends one request to a push gateway.
#[implement(Service)]
#[tracing::instrument(skip(self, dest, request))]
pub async fn send_request<T>(&self, dest: &str, request: T) -> Result<T::IncomingResponse>
where
    T: OutgoingRequest + Metadata<Authentication = NoAuthentication, PathBuilder = SinglePath>,
    T: Debug + Send,
{
    let dest = dest.replace(&self.services.server.config.notification_push_path, "");

    trace!("Push gateway destination: {dest}");

    let http_request = request
        .try_into_http_request::<BytesMut>(&dest, (), ())
        .map_err(|e| {
            err!(BadServerResponse(warn!(
                "Failed to find destination {dest} for push gateway: {e}"
            )))
        })?
        .map(BytesMut::freeze);

    let reqwest_request = reqwest::Request::try_from(http_request)?;

    if let Some(host) = reqwest_request.url().host_str()
        && let Ok(ip) = IPAddress::parse(host)
        && !self.services.client.valid_cidr_range(&ip)
    {
        return Err!(BadServerResponse("Not allowed to send requests to this IP"));
    }

    let mut response = self
        .services
        .client
        .pusher
        .execute(reqwest_request)
        .await
        .inspect_err(|e| warn!("Could not send request to pusher {dest}: {e}"))?;

    if let Some(remote_addr) = response.remote_addr()
        && let Ok(ip) = IPAddress::parse(remote_addr.ip().to_string())
        && !self.services.client.valid_cidr_range(&ip)
    {
        return Err!(BadServerResponse("Not allowed to send requests to this IP"));
    }

    let status = response.status();
    let mut http_response_builder = http::Response::builder()
        .status(status)
        .version(response.version());

    mem::swap(
        response.headers_mut(),
        http_response_builder
            .headers_mut()
            .expect("http::response::Builder is usable"),
    );

    let body = response.bytes().await?;

    if !status.is_success() {
        return Err!(BadServerResponse(warn!(
            "Push gateway {dest} returned unsuccessful HTTP response {status}: {:?}",
            string_from_bytes(&body)
        )));
    }

    T::IncomingResponse::try_from_http_response(
        http_response_builder
            .body(body)
            .expect("reqwest body is valid http body"),
    )
    .map_err(|e| {
        err!(BadServerResponse(warn!(
            "Push gateway {dest} returned invalid response: {e}"
        )))
    })
}

/// Runs one event past one user's push rules, and notifies if they say so.
#[implement(Service)]
#[tracing::instrument(skip(self, user, unread, pusher, ruleset, pdu))]
pub async fn send_push_notice(
    &self,
    user: &UserId,
    unread: UInt,
    pusher: &Pusher,
    ruleset: Ruleset,
    pdu: &PduEvent,
) -> Result {
    let mut notify = None;
    let mut tweaks = Vec::new();

    let power_levels = self.room_power_levels(&pdu.room_id).await;

    for action in self
        .get_actions(
            user,
            &ruleset,
            power_levels,
            &pdu.to_sync_room_event(),
            &pdu.room_id,
        )
        .await
    {
        let n = match action {
            Action::Notify => true,
            Action::SetTweak(tweak) => {
                tweaks.push(tweak.clone());
                continue;
            }
            _ => false,
        };

        if notify.is_some() {
            return Err!(Database(
                r#"Malformed pushrule contains more than one of these actions: ["dont_notify", "notify", "coalesce"]"#
            ));
        }

        notify = Some(n);
    }

    if notify == Some(true) {
        self.send_notice(unread, pusher, tweaks, pdu).await?;
    }

    Ok(())
}

/// A room's power levels, resolved against the rules of its room version.
///
/// A room with no power levels event is not one where nobody has any: from
/// room version 12 the creators hold them implicitly, which is what the rules
/// and the creator list are for.
#[implement(Service)]
async fn room_power_levels(&self, room_id: &RoomId) -> RoomPowerLevels {
    let content = self
        .services
        .state_accessor
        .room_state_get_content::<RoomPowerLevelsEventContent>(
            room_id,
            &StateEventType::RoomPowerLevels,
            "",
        )
        .await
        .ok();

    let (rules, creators) = self
        .services
        .state_accessor
        .power_level_context(room_id)
        .await;

    RoomPowerLevels::new(RoomPowerLevelsSource::from(content), &rules, creators)
}

/// What a user's rules say to do about an event.
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
        .users
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

#[implement(Service)]
#[tracing::instrument(skip(self, unread, pusher, tweaks, event))]
async fn send_notice(
    &self,
    unread: UInt,
    pusher: &Pusher,
    tweaks: Vec<Tweak>,
    event: &PduEvent,
) -> Result {
    let PusherKind::Http(http) = &pusher.kind else {
        return Ok(());
    };

    self.check_gateway_url(&http.url)?;

    let event_id_only = http.format == Some(PushFormat::EventIdOnly);

    let mut device = Device::new(pusher.ids.app_id.clone(), pusher.ids.pushkey.clone());
    device.data.data.clone_from(&http.data);
    device.data.format.clone_from(&http.format);

    if !event_id_only {
        device.tweaks.clone_from(&tweaks);
    }

    let mut notifi = Notification::new(vec![device]);
    notifi.event_id = Some((*event.event_id).to_owned());
    notifi.room_id = Some((*event.room_id).to_owned());

    let badge_disabled = http.data.get("disable_badge_count").is_some()
        || http
            .data
            .get("org.matrix.msc4076.disable_badge_count")
            .is_some();

    if !badge_disabled {
        notifi.counts = NotificationCounts::new(unread, uint!(0));
    }

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
            notifi.user_is_target = event.state_key.as_deref() == Some(event.sender.as_str());
        }

        notifi.sender_display_name = self.services.users.displayname(&event.sender).await.ok();
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

    self.send_request(&http.url, send_event_notification::v1::Request::new(notifi))
        .await?;

    Ok(())
}
