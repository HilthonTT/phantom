use std::{iter, ops::Deref, path::Path, sync::Arc};

use async_trait::async_trait;
use phantom_core::{
    Result, error, implement,
    runtime::config::{Config, validate},
    runtime::server::Server,
};

pub struct Service {
    server: Arc<Server>,
}

const SIGNAL: &str = "SIGUSR1";

#[async_trait]
impl crate::Service for Service {
    fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
        Ok(Arc::new(Self {
            server: args.server.clone(),
        }))
    }

    async fn worker(self: Arc<Self>) -> Result {
        while self.server.running() {
            if self.server.signal.subscribe().recv().await == Ok(SIGNAL)
                && let Err(e) = self.handle_reload()
            {
                error!("failed to reload config: {e}");
            }
        }

        Ok(())
    }

    fn name(&self) -> &str {
        crate::make_name(std::module_path!())
    }
}

impl Deref for Service {
    type Target = Arc<Config>;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.server.config
    }
}

#[implement(Service)]
fn handle_reload(&self) -> Result {
    if self.server.config.admin.config_reload_signal {
        #[cfg(all(feature = "systemd", target_os = "linux"))]
        sd_notify::notify(&[sd_notify::NotifyState::Reloading])
            .expect("failed to notify systemd of reloading state");

        self.reload_running()?;

        #[cfg(all(feature = "systemd", target_os = "linux"))]
        sd_notify::notify(&[sd_notify::NotifyState::Ready])
            .expect("failed to notify systemd of ready state");
    }

    Ok(())
}

/// Reloads the config from where the running server got it: the files and
/// overrides it started with, when the binary recorded them.
#[implement(Service)]
pub fn reload_running(&self) -> Result<Arc<Config>> {
    let new = match self.server.config_source.get() {
        Some(source) => source()?,
        None => Config::load(iter::empty()).and_then(|raw| Config::new(&raw))?,
    };

    self.apply(new)
}

#[implement(Service)]
pub fn reload<'a, I>(&self, paths: I) -> Result<Arc<Config>>
where
    I: Iterator<Item = &'a Path>,
{
    self.apply(Config::load(paths).and_then(|raw| Config::new(&raw))?)
}

#[implement(Service)]
fn apply(&self, new: Config) -> Result<Arc<Config>> {
    let old = self.server.config.clone();

    validate::validate_reload(&old, &new)?;
    self.server.config.update(new)
}
