//! What a capture's filter and closure are handed for each event.

use tracing::Level;
use tracing_core::{Event, span::Current};

use super::{Layer, layer::Value};
use crate::{info, strings::EMPTY};

/// One captured event, with the fields and span scope already recorded.
///
/// Cheap to copy: every field is a borrow of state the layer owns for the
/// duration of the call.
#[derive(Clone, Copy)]
pub struct Data<'a> {
    /// The layer that captured the event.
    pub layer: &'a Layer,

    /// The event itself, for anything the accessors below do not cover.
    pub event: &'a Event<'a>,

    /// The span the event was recorded in, if any.
    pub current: &'a Current,

    /// The event's fields, in the order they were recorded.
    pub values: &'a [Value],

    /// Names of the spans enclosing the event, innermost first.
    pub scope: &'a [&'static str],
}

impl Data<'_> {
    /// Whether the event came from phantom rather than from a dependency.
    #[must_use]
    pub fn our_modules(&self) -> bool {
        self.mod_name().starts_with(info::CRATE_PREFIX)
    }

    /// The level the event was recorded at.
    #[must_use]
    pub fn level(&self) -> Level {
        *self.event.metadata().level()
    }

    /// The module path the event was recorded from.
    #[must_use]
    pub fn mod_name(&self) -> &str {
        self.event.metadata().module_path().unwrap_or(EMPTY)
    }

    /// The name of the innermost span, or the empty string outside one.
    #[must_use]
    pub fn span_name(&self) -> &str {
        self.current.metadata().map_or(EMPTY, |span| span.name())
    }

    /// The event's message, or the empty string for an event with none.
    #[must_use]
    pub fn message(&self) -> &str {
        self.value("message").unwrap_or(EMPTY)
    }

    /// The recorded value of a named field.
    #[must_use]
    pub fn value(&self, name: &str) -> Option<&str> {
        self.values
            .iter()
            .find(|(field, _)| *field == name)
            .map(|(_, value)| value.as_str())
    }
}
