#![cfg(unix)]

use std::{
    env::{args, current_exe},
    os::unix::process::CommandExt,
    process::Command,
};

use phantom_core::{debug, info};

/// Replaces this process with a fresh image of the same executable and
/// arguments, after the server has shut down for a requested restart.
#[cold]
pub(crate) fn restart() -> ! {
    let exe = current_exe().expect("program path must be available");
    let args: Vec<_> = args().skip(1).collect();

    debug!(?exe, ?args, "Restart");
    info!("Restart");

    let error = Command::new(exe).args(args).exec();

    panic!("{error:?}");
}
