use super::*;

/// Convenience trait for adding event type plus state key to state maps.
pub trait EventTypeExt {
    fn with_state_key(self, state_key: impl Into<StateKey>) -> (StateEventType, StateKey);
}

impl EventTypeExt for StateEventType {
    fn with_state_key(self, state_key: impl Into<StateKey>) -> (StateEventType, StateKey) {
        (self, state_key.into())
    }
}

impl EventTypeExt for TimelineEventType {
    fn with_state_key(self, state_key: impl Into<StateKey>) -> (StateEventType, StateKey) {
        (self.to_string().into(), state_key.into())
    }
}

impl<T> EventTypeExt for &T
where
    T: EventTypeExt + Clone,
{
    fn with_state_key(self, state_key: impl Into<StateKey>) -> (StateEventType, StateKey) {
        self.to_owned().with_state_key(state_key)
    }
}
