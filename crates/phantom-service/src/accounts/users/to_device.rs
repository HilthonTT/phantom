use super::*;

impl Service {
    pub async fn add_to_device_event(
        &self,
        sender: &UserId,
        target_user_id: &UserId,
        target_device_id: &DeviceId,
        event_type: &str,
        content: serde_json::Value,
    ) {
        let count = self.services.server_state.next_count().unwrap();

        let key = (target_user_id, target_device_id, count);
        self.db
            .todeviceid_events
            .put(
                key,
                Json(json!({
                    "type": event_type,
                    "sender": sender,
                    "content": content,
                })),
            )
            .ok();
    }

    pub fn get_to_device_events<'a>(
        &'a self,
        user_id: &'a UserId,
        device_id: &'a DeviceId,
        since: Option<u64>,
        to: Option<u64>,
    ) -> impl Stream<Item = Raw<AnyToDeviceEvent>> + Send + 'a {
        type Key<'a> = (&'a str, &'a str, u64);

        let from = (
            user_id,
            device_id,
            since.map_or(0, |since| since.saturating_add(1)),
        );

        self.db
            .todeviceid_events
            .stream_from(&from)
            .ignore_err()
            .ready_take_while(move |((user_id_, device_id_, count), _): &(Key<'_>, _)| {
                user_id.as_str() == *user_id_
                    && device_id.as_str() == *device_id_
                    && to.is_none_or(|to| *count <= to)
            })
            .map(at!(1))
    }

    pub async fn remove_to_device_events<Until>(
        &self,
        user_id: &UserId,
        device_id: &DeviceId,
        until: Until,
    ) where
        Until: Into<Option<u64>> + Send,
    {
        type Key<'a> = (&'a str, &'a str, u64);

        let until = until.into().unwrap_or(u64::MAX);
        let from = (user_id, device_id, until);
        self.db
            .todeviceid_events
            .rev_keys_from(&from)
            .ignore_err()
            .ready_take_while(move |(user_id_, device_id_, _): &Key<'_>| {
                user_id.as_str() == *user_id_ && device_id.as_str() == *device_id_
            })
            .ready_for_each(|key: Key<'_>| {
                self.db.todeviceid_events.del(key).ok();
            })
            .await;
    }
}
