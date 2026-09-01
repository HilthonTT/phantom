//! Runtime reloading of log filters.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use tracing_subscriber::{EnvFilter, reload};

use crate::{Err, Result, error};

/// Forwards to [`reload::Handle::reload`] without naming the handle's type.
///
/// A `reload::Handle<L, S>` carries the type of the subscriber's preceding
/// layers in `S`, which in our case includes unnameable `impl Trait` types, so
/// the handles cannot be stored as themselves. This trait drops `S` so they can
/// be stored as trait objects instead.
///
/// The `S` parameter is gone in the unreleased tracing-subscriber from the
/// master branch[1], which would make this unnecessary — but adopting it means
/// a version of tracing incompatible with the rest of our dependencies.
///
/// [1]: <https://github.com/tokio-rs/tracing/pull/1035/commits/8a87ea52425098d3ef8f56d92358c2f6c144a28f>
pub trait ReloadHandle<L> {
    /// The value currently installed, if the layer still exists.
    fn current(&self) -> Option<L>;

    /// Replaces the installed value.
    fn reload(&self, new_value: L) -> Result<(), reload::Error>;
}

impl<L: Clone, S> ReloadHandle<L> for reload::Handle<L, S> {
    #[inline]
    fn current(&self) -> Option<L> {
        Self::clone_current(self)
    }

    #[inline]
    fn reload(&self, new_value: L) -> Result<(), reload::Error> {
        Self::reload(self, new_value)
    }
}

type Handle = Box<dyn ReloadHandle<EnvFilter> + Send + Sync>;
type HandleMap = HashMap<String, Handle>;

/// The log filters that can be swapped while the server runs, by name.
///
/// Cloning shares the handles: every clone reloads the same layers.
#[derive(Clone, Default)]
pub struct LogLevelReloadHandles {
    handles: Arc<Mutex<HandleMap>>,
}

impl LogLevelReloadHandles {
    /// Registers a layer's reload handle under `name`, replacing any handle
    /// already registered under it.
    pub fn add<H>(&self, name: &str, handle: H)
    where
        H: ReloadHandle<EnvFilter> + Send + Sync + 'static,
    {
        self.handles
            .lock()
            .expect("locked")
            .insert(name.into(), Box::new(handle));
    }

    /// Installs `new_value` on the named layers, or on every layer when `names`
    /// is `None`.
    ///
    /// Reloading is attempted for every named layer even if one fails, and the
    /// first failure is returned once they have all been tried. An unknown name
    /// is an error in itself: the reference implementation quietly reloaded
    /// nothing, which reads as success at the callsite.
    pub fn reload(&self, new_value: &EnvFilter, names: Option<&[&str]>) -> Result {
        let handles = self.handles.lock().expect("locked");

        if let Some(unknown) = names.and_then(|names| {
            names
                .iter()
                .find(|name| !handles.contains_key(**name))
                .copied()
        }) {
            return Err!("There is no log filter named {unknown:?} to reload.");
        }

        let mut failure = None;
        for (name, handle) in handles
            .iter()
            .filter(|(name, _)| names.is_none_or(|names| names.contains(&name.as_str())))
        {
            if let Err(error) = handle.reload(new_value.clone()) {
                error!(%name, "Failed to reload log filter: {error}");
                failure.get_or_insert(error);
            }
        }

        failure.map_or(Ok(()), |error| Err(error.into()))
    }

    /// The filter currently installed on the named layer.
    #[must_use]
    pub fn current(&self, name: &str) -> Option<EnvFilter> {
        self.handles
            .lock()
            .expect("locked")
            .get(name)
            .and_then(|handle| handle.current())
    }

    /// The names every registered layer is known by.
    #[must_use]
    pub fn names(&self) -> Vec<String> {
        let mut names: Vec<_> = self
            .handles
            .lock()
            .expect("locked")
            .keys()
            .cloned()
            .collect();
        names.sort_unstable();
        names
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    /// Stands in for a layer's reload handle, recording what it was asked to
    /// install and optionally refusing to install it.
    struct MockHandle {
        current: Mutex<EnvFilter>,
        reloads: Arc<Mutex<Vec<String>>>,
        fails: bool,
    }

    impl MockHandle {
        fn new(filter: &str, reloads: &Arc<Mutex<Vec<String>>>, fails: bool) -> Self {
            Self {
                current: Mutex::new(EnvFilter::new(filter)),
                reloads: reloads.clone(),
                fails,
            }
        }
    }

    impl ReloadHandle<EnvFilter> for MockHandle {
        fn current(&self) -> Option<EnvFilter> {
            Some(self.current.lock().expect("locked").clone())
        }

        fn reload(&self, new_value: EnvFilter) -> Result<(), reload::Error> {
            self.reloads
                .lock()
                .expect("locked")
                .push(new_value.to_string());

            if self.fails {
                let (_, handle) =
                    reload::Layer::<_, tracing_subscriber::Registry>::new(EnvFilter::new("info"));
                return handle.reload(new_value);
            }

            *self.current.lock().expect("locked") = new_value;

            Ok(())
        }
    }

    fn handles(fails: &[&str]) -> (LogLevelReloadHandles, Arc<Mutex<Vec<String>>>) {
        let reloads = Arc::new(Mutex::new(Vec::new()));
        let handles = LogLevelReloadHandles::default();

        for name in ["console", "capture"] {
            let fails = fails.contains(&name);
            handles.add(name, MockHandle::new("info", &reloads, fails));
        }

        (handles, reloads)
    }

    #[test]
    fn reload_without_names_reloads_every_layer() {
        let (handles, reloads) = handles(&[]);

        handles
            .reload(&EnvFilter::new("debug"), None)
            .expect("reloaded");

        assert_eq!(reloads.lock().expect("locked").len(), 2);
        assert_eq!(
            handles.current("console").map(|f| f.to_string()).as_deref(),
            Some("debug")
        );
        assert_eq!(
            handles.current("capture").map(|f| f.to_string()).as_deref(),
            Some("debug")
        );
    }

    #[test]
    fn reload_with_names_leaves_the_others_alone() {
        let (handles, reloads) = handles(&[]);

        handles
            .reload(&EnvFilter::new("trace"), Some(&["console"]))
            .expect("reloaded");

        assert_eq!(reloads.lock().expect("locked").as_slice(), ["trace"]);
        assert_eq!(
            handles.current("capture").map(|f| f.to_string()).as_deref(),
            Some("info")
        );
    }

    #[test]
    fn an_unknown_name_is_an_error() {
        let (handles, reloads) = handles(&[]);

        let error = handles
            .reload(&EnvFilter::new("debug"), Some(&["console", "nope"]))
            .expect_err("rejected");

        assert!(error.message().contains("\"nope\""), "{error}");
        assert!(
            reloads.lock().expect("locked").is_empty(),
            "nothing is reloaded when a name is wrong"
        );
    }

    #[test]
    fn a_failing_handle_does_not_stop_the_others() {
        let (handles, reloads) = handles(&["console"]);

        handles
            .reload(&EnvFilter::new("debug"), None)
            .expect_err("the failure is reported");

        assert_eq!(
            reloads.lock().expect("locked").len(),
            2,
            "every layer was still tried"
        );
        assert_eq!(
            handles.current("capture").map(|f| f.to_string()).as_deref(),
            Some("debug"),
            "the layer that could reload did"
        );
    }

    #[test]
    fn names_lists_registered_layers() {
        let (handles, _) = handles(&[]);

        assert_eq!(handles.names(), ["capture", "console"]);
        assert!(handles.current("nope").is_none());
    }
}
