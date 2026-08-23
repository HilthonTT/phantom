//! The `tracing` layer feeding active captures.

use std::{fmt, sync::Arc};

use arrayvec::ArrayVec;
use tracing::field::{Field, Visit};
use tracing_core::{Event, Subscriber};
use tracing_subscriber::{layer::Context, registry::LookupSpan};

use super::{Capture, Data, State};

/// A recorded field: its name, and its value rendered to a string.
pub type Value = (&'static str, String);

/// How many fields and enclosing spans are recorded per event. Beyond this the
/// remainder is dropped: an event with more than this many is pathological, and
/// a log layer must not allocate — or panic — on the hot path to accommodate
/// one.
const CAP: usize = 32;

type Values = ArrayVec<Value, CAP>;
type ScopeNames = ArrayVec<&'static str, CAP>;

pub struct Layer {
    state: Arc<State>,
}

impl Layer {
    #[inline]
    #[must_use]
    pub fn new(state: &Arc<State>) -> Self {
        Self {
            state: state.clone(),
        }
    }
}

impl fmt::Debug for Layer {
    #[inline]
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("capture::Layer").finish()
    }
}

impl<S> tracing_subscriber::Layer<S> for Layer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        if !self.state.is_active() {
            return;
        }

        // Recorded once and shared by every capture's filter and closure. The
        // reference implementation recorded twice — fields for the closure and
        // scope names for the filter — so each saw an empty half of the event.
        let mut visitor = Visitor {
            values: Values::new(),
        };

        event.record(&mut visitor);

        let mut scope = ScopeNames::new();
        if let Some(spans) = ctx.event_scope(event) {
            for span in spans {
                if scope.try_push(span.name()).is_err() {
                    break;
                }
            }
        }

        let current = ctx.current_span();
        let data = Data {
            layer: self,
            event,
            current: &current,
            values: &visitor.values,
            scope: &scope,
        };

        self.state
            .active
            .read()
            .expect("locked for reading")
            .iter()
            .filter(|capture| accepts(capture, data))
            .for_each(|capture| {
                let mut closure = capture.closure.lock().expect("locked for writing");
                closure(data);
            });
    }
}

fn accepts(capture: &Capture, data: Data<'_>) -> bool {
    capture.filter.as_ref().is_none_or(|filter| filter(data))
}

struct Visitor {
    values: Values,
}

impl Visit for Visitor {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        _ = self.values.try_push((field.name(), format!("{value:?}")));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        _ = self.values.try_push((field.name(), value.to_owned()));
    }
}
