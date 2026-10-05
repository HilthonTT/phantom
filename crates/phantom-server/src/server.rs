use std::sync::Arc;

use figment::Figment;
use phantom_core::{
    Config, Result, diagnostics::info::version::version, implement, info,
    runtime::server::Server as CoreServer,
};
use phantom_service::Services;
use tokio::sync::Mutex;

use crate::{
    args::{Args, update as update_config},
    logging::TracingFlameGuard,
    runtime::Runtime,
};

/// Server runtime state; complete
pub(crate) struct Server {
    /// Server runtime state; public portion
    pub(crate) server: Arc<CoreServer>,

    pub(crate) services: Mutex<Option<Arc<Services>>>,

    _tracing_flame_guard: TracingFlameGuard,
}

#[implement(Server)]
pub(crate) fn new(args: &Args, runtime: &Runtime) -> Result<Arc<Self>> {
    let handle = runtime.handle();
    let _runtime_guard = handle.enter();

    let config = load_config(args)?;

    let (tracing_flame_guard, log) = crate::logging::init(&config)?;

    info!(
        server_name = %config.server_name,
        database_path = ?config.database.database_path,
        log_levels = %config.logging.log,
        "{}",
        version(),
    );

    Ok(Arc::new(Self {
        server: Arc::new(CoreServer::new(config, Some(handle.clone()), log)),
        services: None.into(),
        _tracing_flame_guard: tracing_flame_guard,
    }))
}

/// Loads and validates the configuration from the files and overrides the
/// arguments name, the same way for the server and for `--health-check`.
pub(crate) fn load_config(args: &Args) -> Result<Config> {
    config_sources(args).and_then(|raw| Config::new(&raw))
}

fn config_sources(args: &Args) -> Result<Figment> {
    let paths = args.config.iter().flatten().map(AsRef::as_ref);

    Config::load(paths).and_then(|raw| update_config(raw, args))
}

#[cfg(test)]
mod tests {
    use std::{
        env::temp_dir,
        fs::{remove_file, write},
        process::id,
    };

    use clap::Parser;

    use super::{Args, load_config};

    #[test]
    fn options_override_the_configuration_file() {
        let path = temp_dir().join(format!("phantom-server-config-{}.toml", id()));

        write(
            &path,
            "[global]\nserver_name = \"file.example\"\ndatabase_path = \"/tmp/phantom\"\n",
        )
        .expect("configuration written");

        let args = Args::parse_from([
            "phantom",
            "-c",
            path.to_str().expect("utf-8 path"),
            "-O",
            r#"server_name="pinned.example""#,
        ]);

        let config = load_config(&args);
        remove_file(&path).expect("configuration removed");

        assert_eq!(config.expect("valid").server_name, "pinned.example");
    }
}
