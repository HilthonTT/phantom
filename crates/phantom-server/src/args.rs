use std::path::PathBuf;

use clap::{ArgAction, Parser, builder::RangedU64ValueParser};
use figment::{Figment, value::Value as FigmentValue};
use phantom_core::{
    Err, Result, diagnostics::info::version::version, err,
    runtime::sys::compute::available_parallelism,
};

/// Commandline arguments
#[derive(Clone, Parser, Debug)]
#[clap(
    about,
    long_about = None,
    name = "phantom",
    version = version(),
)]
pub(crate) struct Args {
    #[arg(short, long)]
    /// Path to the config TOML file (optional)
    pub(crate) config: Option<Vec<PathBuf>>,

    /// Override a configuration variable using TOML 'key=value' syntax
    #[arg(long, short('O'))]
    pub(crate) option: Vec<String>,

    /// Open the database read-only and skip the startup federation burst.
    #[arg(long)]
    pub(crate) read_only: bool,

    /// Probe a running server for liveness and exit; the running server must
    /// share this configuration.
    #[arg(long)]
    pub(crate) health_check: bool,

    /// Execute admin command automatically after startup.
    #[arg(long)]
    pub(crate) execute: Vec<String>,

    /// Override the tokio worker_thread count.
    #[arg(
        long,
        hide(true),
        env = "TOKIO_WORKER_THREADS",
        default_value_t = available_parallelism(),
    )]
    pub(crate) worker_threads: usize,

    /// Override the tokio global_queue_interval.
    #[arg(
        long,
        hide(true),
        env = "TOKIO_GLOBAL_QUEUE_INTERVAL",
        default_value = "192"
    )]
    pub(crate) global_event_interval: u32,

    /// Override the tokio event_interval.
    #[arg(long, hide(true), env = "TOKIO_EVENT_INTERVAL", default_value = "512")]
    pub(crate) kernel_event_interval: u32,

    /// Override the tokio max_io_events_per_tick.
    #[arg(
        long,
        hide(true),
        env = "TOKIO_MAX_IO_EVENTS_PER_TICK",
        default_value = "512"
    )]
    pub(crate) kernel_events_per_tick: usize,

    /// Set the poll histogram bucket size, in microseconds (tokio_unstable).
    ///
    /// Default is 20 microseconds. If the values of the histogram don't
    /// approach zero with the exception of the last bucket, try increasing this
    /// value to e.g. 50 or 100. Inversely, decrease to 10 etc if the histogram
    /// lacks resolution.
    #[arg(
        long,
        hide(true),
        env = "PHANTOM_RUNTIME_POLL_HISTOGRAM_INTERVAL",
        default_value = "20"
    )]
    pub(crate) worker_poll_histogram_interval: u64,

    /// Set the poll histogram bucket count (tokio_unstable).
    ///
    /// Default is 15, and the value must be at least 1.
    #[arg(
        long,
        hide(true),
        env = "PHANTOM_RUNTIME_POLL_HISTOGRAM_BUCKETS",
        default_value = "15",
        value_parser = histogram_buckets_parser()
    )]
    pub(crate) worker_poll_histogram_buckets: usize,

    /// Toggles worker affinity feature.
    #[arg(
        long,
        hide(true),
        env = "PHANTOM_RUNTIME_WORKER_AFFINITY",
        action = ArgAction::Set,
        num_args = 0..=1,
        require_equals(false),
        default_value = "true",
        default_missing_value = "true",
    )]
    pub(crate) worker_affinity: bool,
}

fn histogram_buckets_parser() -> RangedU64ValueParser<usize> {
    RangedU64ValueParser::new().range(1..)
}

/// Parse commandline arguments into structured data
#[must_use]
pub(crate) fn parse() -> Args {
    Args::parse()
}

/// Synthesize any command line options with configuration file options.
pub(crate) fn update(mut config: Figment, args: &Args) -> Result<Figment> {
    if args.read_only {
        config = config.join(("rocksdb_read_only", true));
        config = config.join(("startup_netburst", false));
    }

    // Execute commands after any commands listed in configuration file
    config = config.adjoin(("admin_execute", &args.execute));

    // All other individual overrides can go last in case we have options which
    // set multiple conf items at once and the user still needs granular overrides.
    for option in &args.option {
        let (path, val) = option
            .split_once('=')
            .ok_or_else(|| err!("Missing '=' in -O/--option: {option:?}"))?;

        if path.is_empty() {
            return Err!("Missing key= in -O/--option: {option:?}");
        }

        if val.is_empty() {
            return Err!("Missing =val in -O/--option: {option:?}");
        }

        // The value has to pass for what would appear as a line in the TOML file.
        let val = toml::from_str::<FigmentValue>(option)
            .map_err(|e| err!("Invalid -O/--option {option:?}: {e}"))?;

        // Figment::merge() overrides existing
        config = config.merge((path, val.find(path)));
    }

    Ok(config)
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::{Args, Figment, Parser, Result, update};

    fn updated(argv: &[&str], raw: Figment) -> Result<Figment> {
        update(raw, &Args::parse_from(argv))
    }

    fn long(name: &str) -> OsString {
        let mut argument = OsString::with_capacity(name.len().saturating_add(2));

        argument.push("-");
        argument.push("-");
        argument.push(name);

        argument
    }

    #[test]
    fn options_override_the_configuration() {
        let raw = Figment::new().merge(("server_name", "file.example"));
        let argv = ["phantom", "-O", r#"server_name="pinned.example""#];
        let raw = updated(&argv, raw).expect("accepted");

        assert_eq!(
            raw.find_value("server_name")
                .expect("present")
                .into_string()
                .as_deref(),
            Some("pinned.example"),
        );
    }

    #[test]
    fn malformed_options_are_refused() {
        for option in ["server_name", "=x", "server_name="] {
            updated(&["phantom", "-O", option], Figment::new()).expect_err("refused");
        }
    }

    #[test]
    fn read_only_opens_the_database_read_only() {
        let raw = updated(&["phantom", "--read-only"], Figment::new()).expect("accepted");

        assert_eq!(
            raw.find_value("rocksdb_read_only")
                .expect("present")
                .to_bool(),
            Some(true),
        );
        assert_eq!(
            raw.find_value("startup_netburst")
                .expect("present")
                .to_bool(),
            Some(false),
        );
    }

    #[test]
    fn execute_appends_to_the_configured_commands() {
        let raw = Figment::new().merge(("admin_execute", ["first"]));
        let raw = updated(&["phantom", "--execute", "second"], raw).expect("accepted");

        let commands: Vec<String> = raw.extract_inner("admin_execute").expect("a list");

        assert_eq!(commands, ["first", "second"]);
    }

    #[test]
    fn histogram_bucket_counts_must_be_positive() {
        let option = long("worker-poll-histogram-buckets");

        Args::try_parse_from(["phantom".into(), option.clone(), "0".into()])
            .expect_err("zero histogram buckets rejected");

        Args::try_parse_from(["phantom".into(), option, "1".into()])
            .expect("one histogram bucket accepted");
    }
}
