//! Hot-reloadable configuration.

#![allow(unsafe_code)]

use std::{
    cell::{Cell, RefCell},
    ops::Deref,
    ptr,
    ptr::null_mut,
    sync::{
        Arc,
        atomic::{AtomicPtr, Ordering},
    },
};

use super::Config;
use crate::{Result, implement};

/// The configuration manager is an indirection to reload the configuration for
/// the server while it is running. In order to not burden or clutter the many
/// callsites which query for configuration items, this object implements Deref
/// for the actively loaded configuration.
pub struct Manager {
    active: AtomicPtr<Config>,
}

thread_local! {
    static INDEX: Cell<usize> = 0.into();
    static HANDLE: RefCell<Handles> = const {
        RefCell::new([const { None }; HISTORY])
    };
}

type Handle = Option<Arc<Config>>;
type Handles = [Handle; HISTORY];

const HISTORY: usize = 8;

impl Manager {
    #[must_use]
    pub fn new(config: Config) -> Self {
        let config = Arc::new(config);
        Self {
            active: AtomicPtr::new(Arc::into_raw(config).cast_mut()),
        }
    }
}

impl Drop for Manager {
    fn drop(&mut self) {
        let config = self.active.swap(null_mut(), Ordering::AcqRel);

        unsafe { Arc::from_raw(config) };
    }
}

impl Deref for Manager {
    type Target = Arc<Config>;

    fn deref(&self) -> &Self::Target {
        HANDLE.with_borrow_mut(|handle| self.load(handle))
    }
}

/// Update the active configuration, returning prior configuration.
#[implement(Manager)]
#[tracing::instrument(skip_all)]
pub fn update(&self, config: Config) -> Result<Arc<Config>> {
    let config = Arc::new(config);
    let new = Arc::into_raw(config);
    let old = self.active.swap(new.cast_mut(), Ordering::AcqRel);

    Ok(unsafe { Arc::from_raw(old) })
}

#[implement(Manager)]
fn load(&self, handle: &mut [Option<Arc<Config>>]) -> &'static Arc<Config> {
    let config = self.active.load(Ordering::Acquire);

    if handle[INDEX.get()]
        .as_ref()
        .is_none_or(|handle| !ptr::eq(config, Arc::as_ptr(handle)))
    {
        INDEX.set(INDEX.get().wrapping_add(1).wrapping_rem(HISTORY));
        return load_miss(handle, INDEX.get(), config);
    }

    let config: &Arc<Config> = handle[INDEX.get()]
        .as_ref()
        .expect("handle was already cached for this thread");

    unsafe { std::mem::transmute(config) }
}

#[tracing::instrument(
	name = "miss",
	level = "trace",
	skip_all,
	fields(%index, ?config)
)]
#[allow(clippy::transmute_ptr_to_ptr)]
fn load_miss(
    handle: &mut [Option<Arc<Config>>],
    index: usize,
    config: *const Config,
) -> &'static Arc<Config> {
    let config = unsafe {
        Arc::increment_strong_count(config);
        Arc::from_raw(config)
    };

    unsafe { std::mem::transmute(handle[index].insert(config)) }
}

#[cfg(test)]
mod tests {
    use figment::providers::{Format, Toml};

    use super::*;

    fn config(server_name: &str) -> Config {
        let toml = format!(
            r#"
            [global]
            server_name = "{server_name}"
            database_path = "/var/lib/phantom"
            "#
        );

        Config::new(&figment::Figment::new().merge(Toml::string(&toml).nested()))
            .expect("config is valid")
    }

    #[test]
    fn deref_sees_the_active_config() {
        let manager = Manager::new(config("first.chat"));
        assert_eq!(manager.server_name, "first.chat");
    }

    #[test]
    fn update_swaps_active_and_returns_the_old_config() {
        let manager = Manager::new(config("first.chat"));

        let old = manager.update(config("second.chat")).expect("updated");

        assert_eq!(old.server_name, "first.chat", "prior config is returned");
        assert_eq!(manager.server_name, "second.chat", "deref sees the new one");
    }

    #[test]
    fn repeated_deref_is_cached_and_survives_more_updates_than_history() {
        let manager = Manager::new(config("gen0.chat"));

        for generation in 1..=HISTORY * 2 {
            let name = format!("gen{generation}.chat");
            manager.update(config(&name)).expect("updated");

            assert_eq!(manager.server_name, name);
            assert_eq!(manager.server_name, name);
        }
    }

    #[test]
    fn each_thread_keeps_its_own_handle() {
        let manager = Arc::new(Manager::new(config("shared.chat")));

        let threads: Vec<_> = (0..4)
            .map(|_| {
                let manager = Arc::clone(&manager);
                std::thread::spawn(move || manager.server_name.clone())
            })
            .collect();

        for thread in threads {
            assert_eq!(thread.join().expect("thread did not panic"), "shared.chat");
        }
    }
}
