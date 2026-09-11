use std::{
    borrow::Borrow,
    fmt::{Debug, Display},
    hash::Hash,
    sync::Arc,
};

use ruma::{EventId, MilliSecondsSinceUnixEpoch, RoomId, UserId, events::TimelineEventType};
use serde_json::value::RawValue as RawJsonValue;

pub trait Event {
    type Id: Clone + Debug + Display + Eq + Ord + Hash + Send + Borrow<EventId>;

    fn event_id(&self) -> &Self::Id;

    fn room_id(&self) -> &RoomId;

    fn sender(&self) -> &UserId;

    fn origin_server_ts(&self) -> MilliSecondsSinceUnixEpoch;

    fn event_type(&self) -> &TimelineEventType;

    fn content(&self) -> &RawJsonValue;

    fn state_key(&self) -> Option<&str>;

    fn prev_events(&self) -> impl DoubleEndedIterator<Item = &Self::Id> + Send + '_;

    fn auth_events(&self) -> impl DoubleEndedIterator<Item = &Self::Id> + Send + '_;

    fn redacts(&self) -> Option<&Self::Id>;
}

impl<T: Event> Event for &T {
    type Id = T::Id;

    fn event_id(&self) -> &Self::Id {
        (*self).event_id()
    }

    fn room_id(&self) -> &RoomId {
        (*self).room_id()
    }

    fn sender(&self) -> &UserId {
        (*self).sender()
    }

    fn origin_server_ts(&self) -> MilliSecondsSinceUnixEpoch {
        (*self).origin_server_ts()
    }

    fn event_type(&self) -> &TimelineEventType {
        (*self).event_type()
    }

    fn content(&self) -> &RawJsonValue {
        (*self).content()
    }

    fn state_key(&self) -> Option<&str> {
        (*self).state_key()
    }

    fn prev_events(&self) -> impl DoubleEndedIterator<Item = &Self::Id> + Send + '_ {
        (*self).prev_events()
    }

    fn auth_events(&self) -> impl DoubleEndedIterator<Item = &Self::Id> + Send + '_ {
        (*self).auth_events()
    }

    fn redacts(&self) -> Option<&Self::Id> {
        (*self).redacts()
    }
}

impl<T: Event> Event for Arc<T> {
    type Id = T::Id;

    fn event_id(&self) -> &Self::Id {
        (**self).event_id()
    }

    fn room_id(&self) -> &RoomId {
        (**self).room_id()
    }

    fn sender(&self) -> &UserId {
        (**self).sender()
    }

    fn origin_server_ts(&self) -> MilliSecondsSinceUnixEpoch {
        (**self).origin_server_ts()
    }

    fn event_type(&self) -> &TimelineEventType {
        (**self).event_type()
    }

    fn content(&self) -> &RawJsonValue {
        (**self).content()
    }

    fn state_key(&self) -> Option<&str> {
        (**self).state_key()
    }

    fn prev_events(&self) -> impl DoubleEndedIterator<Item = &Self::Id> + Send + '_ {
        (**self).prev_events()
    }

    fn auth_events(&self) -> impl DoubleEndedIterator<Item = &Self::Id> + Send + '_ {
        (**self).auth_events()
    }

    fn redacts(&self) -> Option<&Self::Id> {
        (**self).redacts()
    }
}
