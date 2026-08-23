//! Ephemeral capture of log events.
//!
//! A [`Capture`] borrows the running server's log stream for as long as its
//! guard is alive, so a one-shot consumer — the admin room's log command, a
//! test asserting on what was logged — can collect events without a second
//! subscriber or a restart. [`Layer`] is installed once at startup and does
//! nothing measurable until a capture is started.

mod data;
mod guard;
mod layer;
mod state;
mod util;

use std::sync::{Arc, Mutex};

pub use self::{
    data::Data,
    guard::Guard,
    layer::{Layer, Value},
    state::State,
    util::{fmt, fmt_html, fmt_markdown},
};

/// Decides whether a captured event is one the consumer asked for.
pub type Filter = dyn Fn(Data<'_>) -> bool + Send + Sync + 'static;

/// Receives each event the filter accepted.
pub type Closure = dyn FnMut(Data<'_>) + Send + Sync + 'static;

/// A capture instance.
///
/// Neither the filter nor the closure may start or stop a capture: both run
/// with the capture list locked, and the log layer is re-entered by anything
/// that logs, so both must also avoid logging themselves.
pub struct Capture {
    state: Arc<State>,
    filter: Option<Box<Filter>>,
    closure: Mutex<Box<Closure>>,
}

impl Capture {
    /// Constructs a capture. Nothing is captured until [`Self::start`] is
    /// called and its guard held.
    #[must_use]
    pub fn new<F, C>(state: &Arc<State>, filter: Option<F>, closure: C) -> Arc<Self>
    where
        F: Fn(Data<'_>) -> bool + Send + Sync + 'static,
        C: FnMut(Data<'_>) + Send + Sync + 'static,
    {
        Arc::new(Self {
            state: state.clone(),
            filter: filter.map(|filter| -> Box<Filter> { Box::new(filter) }),
            closure: Mutex::new(Box::new(closure)),
        })
    }

    /// Starts capturing, until the returned guard is dropped.
    #[must_use]
    pub fn start(self: &Arc<Self>) -> Guard {
        self.state.add(self);

        Guard {
            capture: self.clone(),
        }
    }

    /// Stops capturing. Called for you when the guard drops.
    pub fn stop(self: &Arc<Self>) {
        self.state.del(self);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use tracing::Level;
    use tracing_subscriber::{Registry, layer::SubscriberExt};

    use super::*;

    /// Installs a capture layer as the calling thread's subscriber, runs `f`
    /// with a capture started, and returns what the capture rendered.
    fn captured<F>(filter: Option<fn(Data<'_>) -> bool>, f: F) -> String
    where
        F: FnOnce(),
    {
        let state = Arc::new(State::new());
        let out = Arc::new(Mutex::new(String::new()));
        let capture = Capture::new(&state, filter, fmt_markdown(out.clone()));

        let subscriber = Registry::default().with(Layer::new(&state));
        let _subscriber = tracing::subscriber::set_default(subscriber);

        let guard = capture.start();
        f();
        drop(guard);

        // Nothing recorded after the guard drops may reach the capture.
        tracing::info!("after the guard");

        out.lock().expect("locked").clone()
    }

    #[test]
    fn captures_only_while_the_guard_is_held() {
        let out = captured(None, || tracing::info!("inside"));

        assert!(out.contains("inside"), "{out}");
        assert!(!out.contains("after the guard"), "{out}");
    }

    #[test]
    fn filter_sees_field_values() {
        // The filter is handed the event's recorded fields, which the reference
        // implementation left empty — it recorded them only for the closure.
        let out = captured(
            Some(|data: Data<'_>| data.message().contains("keep")),
            || {
                tracing::info!("keep this");
                tracing::info!("drop that");
            },
        );

        assert!(out.contains("keep this"), "{out}");
        assert!(!out.contains("drop that"), "{out}");
    }

    #[test]
    fn closure_sees_the_span_scope() {
        // Likewise the closure is handed the enclosing spans, which the
        // reference implementation collected only for the filter.
        let scopes: Arc<Mutex<Vec<Vec<String>>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = scopes.clone();

        let state = Arc::new(State::new());
        let capture = Capture::new(
            &state,
            None::<fn(Data<'_>) -> bool>,
            move |data: Data<'_>| {
                let scope = data.scope.iter().map(|name| (*name).to_owned()).collect();
                sink.lock().expect("locked").push(scope);
            },
        );

        let subscriber = Registry::default().with(Layer::new(&state));
        let _subscriber = tracing::subscriber::set_default(subscriber);
        let _guard = capture.start();

        tracing::info_span!("outer").in_scope(|| {
            tracing::info_span!("inner").in_scope(|| tracing::info!("nested"));
        });

        let scopes = scopes.lock().expect("locked");
        assert_eq!(
            scopes.as_slice(),
            [vec!["inner".to_owned(), "outer".to_owned()]],
            "innermost span first"
        );
    }

    #[test]
    fn level_and_message_reach_the_renderer() {
        let out = captured(Some(|data: Data<'_>| data.level() <= Level::WARN), || {
            tracing::info!("chatter");
            tracing::warn!("trouble");
        });

        assert!(out.contains("WARN"), "{out}");
        assert!(out.contains("trouble"), "{out}");
        assert!(!out.contains("chatter"), "{out}");
    }

    #[test]
    fn an_idle_layer_leaves_the_state_inactive() {
        let state = Arc::new(State::new());
        assert!(!state.is_active());

        let capture = Capture::new(&state, None::<fn(Data<'_>) -> bool>, |_| ());
        let guard = capture.start();
        assert!(state.is_active());

        drop(guard);
        assert!(!state.is_active(), "the guard stopped the capture");
    }
}
