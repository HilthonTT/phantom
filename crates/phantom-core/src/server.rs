//! Server-wide runtime state.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, SystemTime},
};

use ruma::OwnedServerName;
use tokio::{runtime, sync::broadcast};

use crate::{Err, Result, config, config::Config, log::Log, metrics::Metrics};

/// Server runtime state; public portion
pub struct Server {
    /// Configured name of server. This is the same as the one in the config
    /// but developers can (and should) reference this string instead.
    pub name: OwnedServerName,

    /// Server-wide configuration instance
    pub config: config::Manager,

    /// Timestamp server was started; used for uptime.
    pub started: SystemTime,

    /// Reload/shutdown pending indicator; server is shutting down. This is an
    /// observable used on shutdown and should not be modified.
    pub stopping: AtomicBool,

    /// Reload/shutdown desired indicator; when false, shutdown is desired. This
    /// is an observable used on shutdown and modifying is not recommended.
    pub reloading: AtomicBool,

    /// Restart desired; when true, restart it desired after shutdown.
    pub restarting: AtomicBool,

    /// Handle to the runtime
    pub runtime: Option<runtime::Handle>,

    /// Reload/shutdown signal
    pub signal: broadcast::Sender<&'static str>,

    /// Logging subsystem state
    pub log: Log,

    /// Metrics subsystem state
    pub metrics: Metrics,
}

impl Server {
    #[must_use]
    pub fn new(config: Config, runtime: Option<runtime::Handle>, log: Log) -> Self {
        Self {
            name: config
                .server_name
                .parse()
                .expect("`server_name` was validated by config::check"),
            config: config::Manager::new(config),
            started: SystemTime::now(),
            stopping: AtomicBool::new(false),
            reloading: AtomicBool::new(false),
            restarting: AtomicBool::new(false),
            runtime: runtime.clone(),
            signal: broadcast::channel::<&'static str>(1).0,
            log,
            metrics: Metrics::new(runtime),
        }
    }

    /// Swaps the server's modules for freshly built ones without dropping
    /// connections.
    ///
    /// Reloading relies on the dynamic module loading phantom does not
    /// implement yet, so this always fails; it exists so callers — the admin
    /// room, a signal handler — can offer the command and report why rather
    /// than not having one.
    pub fn reload(&self) -> Result {
        Err!("Reloading is not supported by this build; restart instead.")
    }

    /// Shuts the server down, to be started again by the supervising process.
    pub fn restart(&self) -> Result {
        if self.restarting.swap(true, Ordering::AcqRel) {
            return Err!("Restart already in progress");
        }

        self.shutdown().inspect_err(|_| {
            self.restarting.store(false, Ordering::Release);
        })
    }

    /// Begins an orderly shutdown.
    pub fn shutdown(&self) -> Result {
        if self.stopping.swap(true, Ordering::AcqRel) {
            return Err!("Shutdown already in progress");
        }

        self.signal("SIGTERM").inspect_err(|_| {
            self.stopping.store(false, Ordering::Release);
        })
    }

    /// Broadcasts a signal to everything waiting on one.
    ///
    /// Fails when nothing is listening: a shutdown nobody will act on is a
    /// failed shutdown, and the callers above roll their state back on it.
    pub fn signal(&self, sig: &'static str) -> Result {
        if let Err(e) = self.signal.send(sig) {
            return Err!("Failed to send signal: {e}");
        }

        Ok(())
    }

    /// Resolves once the server is shutting down.
    #[inline]
    pub async fn until_shutdown(self: &Arc<Self>) {
        // Subscribed before the first check, so a signal sent between the two
        // is queued rather than missed. Re-subscribing inside the loop, as the
        // reference did, reopens that window on every iteration.
        let mut signal = self.signal.subscribe();

        while self.running() {
            if matches!(
                signal.recv().await,
                Err(broadcast::error::RecvError::Closed)
            ) {
                break;
            }
        }
    }

    /// How long the server has been up.
    ///
    /// Zero if the system clock has moved backwards past the start time.
    #[inline]
    #[must_use]
    pub fn uptime(&self) -> Duration {
        self.started.elapsed().unwrap_or_default()
    }

    #[inline]
    pub fn runtime(&self) -> &runtime::Handle {
        self.runtime
            .as_ref()
            .expect("runtime handle available in Server")
    }

    /// `Ok` while the server is running, so a long operation can bail out of a
    /// loop with `?` once shutdown starts.
    #[inline]
    pub fn check_running(&self) -> Result {
        use std::{io, io::ErrorKind::Interrupted};

        self.running()
            .then_some(())
            .ok_or_else(|| io::Error::new(Interrupted, "Server shutting down"))
            .map_err(Into::into)
    }

    #[inline]
    #[must_use]
    pub fn running(&self) -> bool {
        !self.is_stopping()
    }

    #[inline]
    #[must_use]
    pub fn is_stopping(&self) -> bool {
        self.stopping.load(Ordering::Relaxed)
    }

    #[inline]
    #[must_use]
    pub fn is_reloading(&self) -> bool {
        self.reloading.load(Ordering::Relaxed)
    }

    #[inline]
    #[must_use]
    pub fn is_restarting(&self) -> bool {
        self.restarting.load(Ordering::Relaxed)
    }

    /// Whether `name` is this server, i.e. whether a room or user is local.
    #[inline]
    #[must_use]
    pub fn is_ours(&self, name: &str) -> bool {
        name == self.config.server_name
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use figment::providers::{Format, Toml};

    use super::*;
    use crate::log::{LogLevelReloadHandles, capture};

    fn server() -> Arc<Server> {
        let toml = r#"
            [global]
            server_name = "phantom.chat"
            database_path = "/var/lib/phantom"
            "#;

        let config = Config::new(&figment::Figment::new().merge(Toml::string(toml).nested()))
            .expect("config is valid");

        let log = Log {
            reload: LogLevelReloadHandles::default(),
            capture: Arc::new(capture::State::new()),
        };

        Arc::new(Server::new(config, None, log))
    }

    #[test]
    fn a_new_server_is_running() {
        let server = server();

        assert_eq!(server.name.as_str(), "phantom.chat");
        assert!(server.running());
        assert!(server.check_running().is_ok());
        assert!(!server.is_stopping() && !server.is_reloading() && !server.is_restarting());
    }

    #[test]
    fn shutdown_is_idempotent_and_observable() {
        let server = server();
        let mut signal = server.signal.subscribe();

        server.shutdown().expect("shutdown started");

        assert!(server.is_stopping());
        assert!(!server.running());
        assert!(server.check_running().is_err());
        assert_eq!(signal.try_recv().expect("signalled"), "SIGTERM");

        let again = server.shutdown().expect_err("already shutting down");
        assert!(again.message().contains("already in progress"), "{again}");
    }

    #[test]
    fn restart_shuts_down_and_records_the_intent() {
        let server = server();
        let _signal = server.signal.subscribe();

        server.restart().expect("restart started");

        assert!(server.is_restarting());
        assert!(server.is_stopping());

        let again = server.restart().expect_err("already restarting");
        assert!(again.message().contains("already in progress"), "{again}");
    }

    #[test]
    fn a_failed_shutdown_does_not_leave_the_server_marked_stopping() {
        let server = server();

        // With no subscriber the broadcast has nowhere to send, which is what
        // the rollback in `shutdown` is for.
        server.shutdown().expect_err("no receivers");

        assert!(server.running(), "the stopping flag was rolled back");
    }

    #[test]
    fn reload_reports_that_it_is_unsupported() {
        let error = server().reload().expect_err("unsupported");

        assert!(error.message().contains("restart"), "{error}");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn until_shutdown_returns_once_shutdown_begins() {
        let server = server();
        let waiter = server.clone();
        let waiting = tokio::spawn(async move { waiter.until_shutdown().await });

        // Yield until the task has subscribed, then signal it.
        tokio::task::yield_now().await;
        while server.shutdown().is_err() {
            tokio::task::yield_now().await;
        }

        waiting.await.expect("the waiter observed the shutdown");
    }

    #[test]
    fn is_ours_matches_the_configured_name() {
        let server = server();

        assert!(server.is_ours("phantom.chat"));
        assert!(!server.is_ours("elsewhere.chat"));
    }
}
