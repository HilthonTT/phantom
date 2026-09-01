//! Temporary silencing of console logging.

use std::sync::Arc;

use super::EnvFilter;
use crate::{error, server::Server};

/// The layer console output goes through.
const HANDLE: &str = "console";

/// Silences the console for as long as it is held.
///
/// Used where phantom writes to the terminal itself — the interactive console
/// command line — and a log line arriving mid-prompt would corrupt the display.
pub struct Suppress {
    server: Arc<Server>,

    /// The filter to put back, or `None` when suppression never took effect and
    /// there is nothing to restore.
    restore: Option<EnvFilter>,
}

impl Suppress {
    #[must_use]
    pub fn new(server: &Arc<Server>) -> Self {
        let suppress = EnvFilter::default();

        let restore = server
            .log
            .reload
            .current(HANDLE)
            .unwrap_or_else(|| EnvFilter::try_new(&server.config.log).unwrap_or_default());

        let restore = server
            .log
            .reload
            .reload(&suppress, Some(&[HANDLE]))
            .inspect_err(|error| error!("Failed to suppress console logging: {error}"))
            .is_ok()
            .then_some(restore);

        Self {
            server: server.clone(),
            restore,
        }
    }
}

impl Drop for Suppress {
    fn drop(&mut self) {
        let Some(restore) = self.restore.take() else {
            return;
        };

        _ = self
            .server
            .log
            .reload
            .reload(&restore, Some(&[HANDLE]))
            .inspect_err(|error| error!("Failed to restore console logging: {error}"));
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use figment::providers::{Format, Toml};
    use tracing_subscriber::reload;

    use super::{
        super::{Log, LogLevelReloadHandles, ReloadHandle, capture},
        *,
    };
    use crate::{Config, Result};

    /// Stands in for the console layer's reload handle.
    struct MockHandle(Mutex<EnvFilter>);

    impl ReloadHandle<EnvFilter> for MockHandle {
        fn current(&self) -> Option<EnvFilter> {
            Some(self.0.lock().expect("locked").clone())
        }

        fn reload(&self, new_value: EnvFilter) -> Result<(), reload::Error> {
            *self.0.lock().expect("locked") = new_value;
            Ok(())
        }
    }

    fn server(register_console: bool) -> Arc<Server> {
        let toml = r#"
            [global]
            server_name = "phantom.chat"
            database_path = "/var/lib/phantom"
            log = "warn"
            "#;

        let config = Config::new(&figment::Figment::new().merge(Toml::string(toml).nested()))
            .expect("config is valid");

        let reload = LogLevelReloadHandles::default();
        if register_console {
            reload.add(HANDLE, MockHandle(Mutex::new(EnvFilter::new("warn"))));
        }

        let log = Log {
            reload,
            capture: Arc::new(capture::State::new()),
        };

        Arc::new(Server::new(config, None, log))
    }

    fn console_filter(server: &Arc<Server>) -> Option<String> {
        server
            .log
            .reload
            .current(HANDLE)
            .map(|filter| filter.to_string())
    }

    #[test]
    fn suppresses_the_console_and_restores_it() {
        let server = server(true);
        assert_eq!(console_filter(&server).as_deref(), Some("warn"));

        {
            let _suppress = Suppress::new(&server);
            assert_eq!(
                console_filter(&server).as_deref(),
                Some(""),
                "an empty filter enables nothing"
            );
        }

        assert_eq!(
            console_filter(&server).as_deref(),
            Some("warn"),
            "the prior filter is put back"
        );
    }

    #[test]
    fn an_unsuppressible_console_is_not_fatal() {
        let server = server(false);

        drop(Suppress::new(&server));
    }
}
