//! The rest of the timeline write path, not yet ported.
//!
//! This file is deliberately not a module of `timeline`: nothing declares
//! `mod pending_write_path;`, so it is not compiled. It is conduwuit's write
//! path as pasted, kept close to verbatim so it can be ported function by
//! function rather than rewritten from memory later.
//!
//! `append_pdu`, `append_incoming_pdu` and `redact_pdu` have been ported and
//! live in [`append`](super::append). What is left here is the two halves of
//! sending an event of this server's own, and backfill:
//!
//! - `create_hash_and_sign_event` — builds a PDU from a `PduBuilder`, fills in
//!   its auth events and prev events, hashes and signs it
//! - `build_and_append_pdu` — the above, then `append_pdu`, then telling the
//!   other servers in the room
//! - `backfill_if_required` / `backfill_pdu` — asking another server for the
//!   history before what this server holds
//! - `check_pdu_for_admin_room` — the extra rules that keep the admin room
//!   from being left or banned empty
//!
//! Of these, only backfill is still waiting on a service: it needs
//! `rooms::event_handler` to place the events it fetches. The other three are
//! waiting on the porting itself — adapting conduwuit's calls to phantom's
//! names, and to a newer ruma whose event and signing APIs have moved since.
//!
//! The imports the parked code needs are conduwuit's, not phantom's, and are
//! left alone for the same reason the bodies are:
//!
//! ```ignore
//! use conduwuit::{
//!     Err, Error, Result, Server, at, debug, debug_warn, err, error, implement, info,
//!     matrix::{
//!         Event,
//!         pdu::{EventHash, PduBuilder, PduCount, PduEvent, gen_event_id},
//!         state_res::{self, RoomVersion},
//!     },
//!     utils::{
//!         self, IterStream, MutexMap, MutexMapGuard, ReadyExt, future::TryExtExt,
//!         stream::TryIgnore,
//!     },
//!     validated, warn,
//! };
//! ```
//!
//! Along with the `ExtractEventId` deserializer, which only this half uses.

    pub async fn create_hash_and_sign_event(
        &self,
        pdu_builder: PduBuilder,
        sender: &UserId,
        room_id: &RoomId,
        _mutex_lock: &RoomMutexGuard,
    ) -> Result<(PduEvent, CanonicalJsonObject)> {
        let PduBuilder {
            event_type,
            content,
            unsigned,
            state_key,
            redacts,
            timestamp,
        } = pdu_builder;

        let prev_events: Vec<OwnedEventId> = self
            .services
            .state
            .get_forward_extremities(room_id)
            .take(20)
            .map(Into::into)
            .collect()
            .await;

        let room_version_id = self
            .services
            .state
            .get_room_version(room_id)
            .await
            .or_else(|_| {
                if event_type == TimelineEventType::RoomCreate {
                    let content: RoomCreateEventContent = serde_json::from_str(content.get())?;
                    Ok(content.room_version)
                } else {
                    Err(Error::InconsistentRoomState(
                        "non-create event for room of unknown version",
                        room_id.to_owned(),
                    ))
                }
            })?;

        let room_version = RoomVersion::new(&room_version_id).expect("room version is supported");

        let auth_events = self
            .services
            .state
            .get_auth_events(room_id, &event_type, sender, state_key.as_deref(), &content)
            .await?;

        let depth = prev_events
            .iter()
            .stream()
            .map(Ok)
            .and_then(|event_id| self.get_pdu(event_id))
            .and_then(|pdu| future::ok(pdu.depth))
            .ignore_err()
            .ready_fold(uint!(0), cmp::max)
            .await
            .saturating_add(uint!(1));

        let mut unsigned = unsigned.unwrap_or_default();

        if let Some(state_key) = &state_key {
            if let Ok(prev_pdu) = self
                .services
                .state_accessor
                .room_state_get(room_id, &event_type.to_string().into(), state_key)
                .await
            {
                unsigned.insert("prev_content".to_owned(), prev_pdu.get_content_as_value());
                unsigned.insert(
                    "prev_sender".to_owned(),
                    serde_json::to_value(&prev_pdu.sender).expect("UserId::to_value always works"),
                );
                unsigned.insert(
                    "replaces_state".to_owned(),
                    serde_json::to_value(&prev_pdu.event_id).expect("EventId is valid json"),
                );
            }
        }

        let mut pdu = PduEvent {
            event_id: ruma::event_id!("$thiswillbefilledinlater").into(),
            room_id: room_id.to_owned(),
            sender: sender.to_owned(),
            origin: None,
            origin_server_ts: timestamp.map_or_else(
                || {
                    utils::millis_since_unix_epoch()
                        .try_into()
                        .expect("u64 fits into UInt")
                },
                |ts| ts.get(),
            ),
            kind: event_type,
            content,
            state_key,
            prev_events,
            depth,
            auth_events: auth_events
                .values()
                .map(|pdu| pdu.event_id.clone())
                .collect(),
            redacts,
            unsigned: if unsigned.is_empty() {
                None
            } else {
                Some(to_raw_value(&unsigned).expect("to_raw_value always works"))
            },
            hashes: EventHash {
                sha256: "aaa".to_owned(),
            },
            signatures: None,
        };

        let auth_fetch = |k: &StateEventType, s: &str| {
            let key = (k.clone(), s.into());
            ready(auth_events.get(&key))
        };

        let auth_check = state_res::auth_check(
            &room_version,
            &pdu,
            None,
            auth_fetch,
        )
        .await
        .map_err(|e| err!(Request(Forbidden(warn!("Auth check failed: {e:?}")))))?;

        if !auth_check {
            return Err!(Request(Forbidden("Event is not authorized.")));
        }

        let mut pdu_json = utils::to_canonical_object(&pdu).map_err(|e| {
            err!(Request(BadJson(warn!(
                "Failed to convert PDU to canonical JSON: {e}"
            ))))
        })?;

        match room_version_id {
            RoomVersionId::V1 | RoomVersionId::V2 => {}
            _ => {
                pdu_json.remove("event_id");
            }
        }

        pdu_json.insert(
            "origin".to_owned(),
            to_canonical_value(self.services.globals.server_name())
                .expect("server name is a valid CanonicalJsonValue"),
        );

        if let Err(e) = self
            .services
            .server_keys
            .hash_and_sign_event(&mut pdu_json, &room_version_id)
        {
            return match e {
                Error::Signatures(ruma::signatures::Error::PduSize) => {
                    Err!(Request(TooLarge(
                        "Message/PDU is too long (exceeds 65535 bytes)"
                    )))
                }
                _ => Err!(Request(Unknown(warn!("Signing event failed: {e}")))),
            };
        }

        pdu.event_id = gen_event_id(&pdu_json, &room_version_id)?;

        pdu_json.insert(
            "event_id".into(),
            CanonicalJsonValue::String(pdu.event_id.clone().into()),
        );

        let _shorteventid = self
            .services
            .short
            .get_or_create_shorteventid(&pdu.event_id)
            .await;

        Ok((pdu, pdu_json))
    }

    /// Creates a new persisted data unit and adds it to a room. This function
    /// takes a roomid_mutex_state, meaning that only this function is able to
    /// mutate the room state.
    #[tracing::instrument(skip(self, state_lock), level = "debug")]
    pub async fn build_and_append_pdu(
        &self,
        pdu_builder: PduBuilder,
        sender: &UserId,
        room_id: &RoomId,
        state_lock: &RoomMutexGuard,
    ) -> Result<OwnedEventId> {
        let (pdu, pdu_json) = self
            .create_hash_and_sign_event(pdu_builder, sender, room_id, state_lock)
            .await?;

        if self.services.admin.is_admin_room(&pdu.room_id).await {
            self.check_pdu_for_admin_room(&pdu, sender).boxed().await?;
        }

        if pdu.kind == TimelineEventType::RoomRedaction {
            use RoomVersionId::*;
            match self.services.state.get_room_version(&pdu.room_id).await? {
                V1 | V2 | V3 | V4 | V5 | V6 | V7 | V8 | V9 | V10 => {
                    if let Some(redact_id) = &pdu.redacts {
                        if !self
                            .services
                            .state_accessor
                            .user_can_redact(redact_id, &pdu.sender, &pdu.room_id, false)
                            .await?
                        {
                            return Err!(Request(Forbidden("User cannot redact this event.")));
                        }
                    }
                }
                _ => {
                    let content: RoomRedactionEventContent = pdu.get_content()?;
                    if let Some(redact_id) = &content.redacts {
                        if !self
                            .services
                            .state_accessor
                            .user_can_redact(redact_id, &pdu.sender, &pdu.room_id, false)
                            .await?
                        {
                            return Err!(Request(Forbidden("User cannot redact this event.")));
                        }
                    }
                }
            }
        }

        if pdu.kind == TimelineEventType::RoomMember {
            let content: RoomMemberEventContent = pdu.get_content()?;

            if content.join_authorized_via_users_server.is_some()
                && content.membership != MembershipState::Join
            {
                return Err!(Request(BadJson(
                    "join_authorised_via_users_server is only for member joins"
                )));
            }

            if content
                .join_authorized_via_users_server
                .as_ref()
                .is_some_and(|authorising_user| {
                    !self.services.globals.user_is_local(authorising_user)
                })
            {
                return Err!(Request(InvalidParam(
                    "Authorising user does not belong to this homeserver"
                )));
            }
        }

        let statehashid = self.services.state.append_to_state(&pdu).await?;

        let pdu_id = self
            .append_pdu(
                &pdu,
                pdu_json,
                once(pdu.event_id.borrow()),
                state_lock,
            )
            .boxed()
            .await?;

        self.services
            .state
            .set_room_state(&pdu.room_id, statehashid, state_lock);

        let mut servers: HashSet<OwnedServerName> = self
            .services
            .state_cache
            .room_servers(&pdu.room_id)
            .map(ToOwned::to_owned)
            .collect()
            .await;

        if pdu.kind == TimelineEventType::RoomMember {
            if let Some(state_key_uid) = &pdu
                .state_key
                .as_ref()
                .and_then(|state_key| UserId::parse(state_key.as_str()).ok())
            {
                servers.insert(state_key_uid.server_name().to_owned());
            }
        }

        servers.remove(self.services.globals.server_name());

        self.services
            .sending
            .send_pdu_servers(servers.iter().map(AsRef::as_ref).stream(), &pdu_id)
            .await?;

        Ok(pdu.event_id)
    }

    #[tracing::instrument(name = "backfill", level = "debug", skip(self))]
    pub async fn backfill_if_required(&self, room_id: &RoomId, from: PduCount) -> Result<()> {
        if self
            .services
            .state_cache
            .room_joined_count(room_id)
            .await
            .is_ok_and(|count| count <= 1)
            && !self
                .services
                .state_accessor
                .is_world_readable(room_id)
                .await
        {
            return Ok(());
        }

        let first_pdu = self
            .first_item_in_room(room_id)
            .await
            .expect("Room is not empty");

        if first_pdu.0 < from {
            return Ok(());
        }

        let power_levels: RoomPowerLevelsEventContent = self
            .services
            .state_accessor
            .room_state_get_content(room_id, &StateEventType::RoomPowerLevels, "")
            .await
            .unwrap_or_default();

        let room_mods = power_levels.users.iter().filter_map(|(user_id, level)| {
            if level > &power_levels.users_default && !self.services.globals.user_is_local(user_id)
            {
                Some(user_id.server_name())
            } else {
                None
            }
        });

        let canonical_room_alias_server = once(
            self.services
                .state_accessor
                .get_canonical_alias(room_id)
                .await,
        )
        .filter_map(Result::ok)
        .map(|alias| alias.server_name().to_owned())
        .stream();

        let mut servers = room_mods
            .stream()
            .map(ToOwned::to_owned)
            .chain(canonical_room_alias_server)
            .chain(
                self.services
                    .server
                    .config
                    .trusted_servers
                    .iter()
                    .map(ToOwned::to_owned)
                    .stream(),
            )
            .ready_filter(|server_name| !self.services.globals.server_is_ours(server_name))
            .filter_map(|server_name| async move {
                self.services
                    .state_cache
                    .server_in_room(&server_name, room_id)
                    .await
                    .then_some(server_name)
            })
            .boxed();

        while let Some(ref backfill_server) = servers.next().await {
            info!("Asking {backfill_server} for backfill");
            let response = self
                .services
                .sending
                .send_federation_request(
                    backfill_server,
                    federation::backfill::get_backfill::v1::Request {
                        room_id: room_id.to_owned(),
                        v: vec![first_pdu.1.event_id.clone()],
                        limit: uint!(100),
                    },
                )
                .await;
            match response {
                Ok(response) => {
                    for pdu in response.pdus {
                        if let Err(e) = self.backfill_pdu(backfill_server, pdu).boxed().await {
                            debug_warn!("Failed to add backfilled pdu in room {room_id}: {e}");
                        }
                    }
                    return Ok(());
                }
                Err(e) => {
                    warn!("{backfill_server} failed to provide backfill for room {room_id}: {e}");
                }
            }
        }

        info!("No servers could backfill, but backfill was needed in room {room_id}");
        Ok(())
    }

    #[tracing::instrument(skip(self, pdu), level = "debug")]
    pub async fn backfill_pdu(&self, origin: &ServerName, pdu: Box<RawJsonValue>) -> Result<()> {
        let (room_id, event_id, value) =
            self.services.event_handler.parse_incoming_pdu(&pdu).await?;

        let mutex_lock = self
            .services
            .event_handler
            .mutex_federation
            .lock(&room_id)
            .await;

        if let Ok(pdu_id) = self.get_pdu_id(&event_id).await {
            debug!("We already know {event_id} at {pdu_id:?}");
            return Ok(());
        }

        self.services
            .event_handler
            .handle_incoming_pdu(origin, &room_id, &event_id, value, false)
            .boxed()
            .await?;

        let value = self.get_pdu_json(&event_id).await?;

        let pdu = self.get_pdu(&event_id).await?;

        let shortroomid = self.services.short.get_shortroomid(&room_id).await?;

        let insert_lock = self.mutex_insert.lock(&room_id).await;

        let count: i64 = self.services.globals.next_count().unwrap().try_into()?;

        let pdu_id: RawPduId = PduId {
            shortroomid,
            shorteventid: PduCount::Backfilled(validated!(0 - count)),
        }
        .into();

        self.db.prepend_backfill_pdu(&pdu_id, &event_id, &value);

        drop(insert_lock);

        if pdu.kind == TimelineEventType::RoomMessage {
            let content: ExtractBody = pdu.get_content()?;
            if let Some(body) = content.body {
                self.services.search.index_pdu(shortroomid, &pdu_id, &body);
            }
        }
        drop(mutex_lock);

        debug!("Prepended backfill pdu");
        Ok(())
    }
}

#[implement(Service)]
#[tracing::instrument(skip_all, level = "debug")]
async fn check_pdu_for_admin_room(&self, pdu: &PduEvent, sender: &UserId) -> Result<()> {
    match &pdu.kind {
        TimelineEventType::RoomEncryption => {
            return Err!(Request(Forbidden(error!(
                "Encryption not supported in admins room."
            ))));
        }
        TimelineEventType::RoomMember => {
            let target = pdu
                .state_key()
                .filter(|v| v.starts_with('@'))
                .unwrap_or(sender.as_str());

            let server_user = &self.services.globals.server_user.to_string();

            let content: RoomMemberEventContent = pdu.get_content()?;
            match content.membership {
                MembershipState::Leave => {
                    if target == server_user {
                        return Err!(Request(Forbidden(error!(
                            "Server user cannot leave the admins room."
                        ))));
                    }

                    let count = self
                        .services
                        .state_cache
                        .room_members(&pdu.room_id)
                        .ready_filter(|user| self.services.globals.user_is_local(user))
                        .ready_filter(|user| *user != target)
                        .boxed()
                        .count()
                        .await;

                    if count < 2 {
                        return Err!(Request(Forbidden(error!(
                            "Last admin cannot leave the admins room."
                        ))));
                    }
                }

                MembershipState::Ban if pdu.state_key().is_some() => {
                    if target == server_user {
                        return Err!(Request(Forbidden(error!(
                            "Server cannot be banned from admins room."
                        ))));
                    }

                    let count = self
                        .services
                        .state_cache
                        .room_members(&pdu.room_id)
                        .ready_filter(|user| self.services.globals.user_is_local(user))
                        .ready_filter(|user| *user != target)
                        .boxed()
                        .count()
                        .await;

                    if count < 2 {
                        return Err!(Request(Forbidden(error!(
                            "Last admin cannot be banned from admins room."
                        ))));
                    }
                }
                _ => {}
            }
        }
        _ => {}
    }

    Ok(())
}
