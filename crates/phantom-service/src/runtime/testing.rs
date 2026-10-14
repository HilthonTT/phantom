//! The service graph over a real database in a temporary directory, for
//! tests that need more than one service at a time.
//!
//! Only `Services::build` runs: no worker starts and no migration runs, so a
//! test sees exactly the state it writes, and builds what else it needs.

use std::sync::Arc;

use figment::{
    Figment,
    providers::{Format, Toml},
};
use phantom_core::{
    Config,
    diagnostics::log::{Log, LogLevelReloadHandles, capture},
    runtime::server::Server,
};
use tempfile::TempDir;

use super::Services;

pub(crate) const SERVER_NAME: &str = "phantom.test";

pub(crate) struct TestServices {
    pub(crate) services: Arc<Services>,
    _dir: TempDir,
}

impl std::ops::Deref for TestServices {
    type Target = Services;

    fn deref(&self) -> &Services {
        &self.services
    }
}

/// Builds every service against a fresh database; `extra` is more TOML for
/// the `[global]` table.
pub(crate) fn build(extra: &str) -> TestServices {
    let dir = tempfile::tempdir().expect("a temporary directory");

    TestServices {
        services: build_at(&dir, extra),
        _dir: dir,
    }
}

/// Builds every service against the database already in `dir`, so a test
/// can drop the graph and build it again to see what survives a restart.
pub(crate) fn build_at(dir: &TempDir, extra: &str) -> Arc<Services> {
    Services::build(server(dir, extra)).expect("the service graph builds")
}

/// A server configured for the database in `dir`, for tests that open the
/// database alone rather than the whole service graph.
pub(crate) fn server(dir: &TempDir, extra: &str) -> Arc<Server> {
    let path = dir.path().display();

    // The pool settings keep each test database to one worker thread, as the
    // database crate's own tests do; tests run in parallel, and the default
    // sizing would open far more threads than a test host allows.
    let toml = format!(
        r#"
        [global]
        server_name = "{SERVER_NAME}"
        database_path = "{path}"
        db_pool_workers = 1
        db_pool_workers_limit = 1
        db_pool_queue_mult = 1
        db_cache_capacity_mb = 8.0
        db_write_buffer_capacity_mb = 8.0
        {extra}
        "#
    );

    let config = Config::new(&Figment::new().merge(Toml::string(&toml).nested()))
        .expect("a valid test configuration");

    let log = Log {
        reload: LogLevelReloadHandles::default(),
        capture: Arc::new(capture::State::new()),
    };

    Arc::new(Server::new(config, None, log))
}
