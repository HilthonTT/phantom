use std::sync::Arc;

use phantom_core::{debug_error, warn};
use tokio::signal;

use crate::server::Server;

/// Turns process signals into server signals until shutdown: SIGINT, SIGQUIT
/// and SIGTERM stop the server, while SIGUSR1 and SIGUSR2 are broadcast for
/// the services to act on.
#[cfg(unix)]
#[tracing::instrument(skip_all)]
pub(crate) async fn enable(server: Arc<Server>) {
    use phantom_core::trace;
    use signal::unix::{self, SignalKind};

    let mut quit = unix::signal(SignalKind::quit()).expect("SIGQUIT handler");
    let mut term = unix::signal(SignalKind::terminate()).expect("SIGTERM handler");
    let mut usr1 = unix::signal(SignalKind::user_defined1()).expect("SIGUSR1 handler");
    let mut usr2 = unix::signal(SignalKind::user_defined2()).expect("SIGUSR2 handler");
    loop {
        trace!("Installed signal handlers");
        let sig: &'static str;
        tokio::select! {
            () = server.server.until_shutdown() => break,
            _ = signal::ctrl_c() => { sig = "SIGINT"; },
            _ = quit.recv() => { sig = "SIGQUIT"; },
            _ = term.recv() => { sig = "SIGTERM"; },
            _ = usr1.recv() => { sig = "SIGUSR1"; },
            _ = usr2.recv() => { sig = "SIGUSR2"; },
        }

        warn!("Received {sig}");
        let result = if matches!(sig, "SIGINT" | "SIGQUIT" | "SIGTERM") {
            server.server.shutdown()
        } else {
            server.server.signal(sig)
        };

        if let Err(e) = result {
            debug_error!(?sig, "signal: {e}");
        }
    }
}

#[cfg(not(unix))]
#[tracing::instrument(skip_all)]
pub(crate) async fn enable(server: Arc<Server>) {
    loop {
        tokio::select! {
            () = server.server.until_shutdown() => break,
            _ = signal::ctrl_c() => {
                warn!("Received Ctrl+C");
                if let Err(e) = server.server.shutdown() {
                    debug_error!("shutdown: {e}");
                }
            },
        }
    }
}
