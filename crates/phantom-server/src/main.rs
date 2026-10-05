#![recursion_limit = "256"]

mod args;
mod health;
mod logging;
mod restart;
mod runtime;
mod serve;
mod server;
mod signal;

use std::sync::Arc;

use phantom_core::{Error, Result, debug_info, defer, error, info};

use crate::{runtime::Runtime, server::Server};

fn main() -> Result {
    let args = args::parse();

    if args.health_check {
        return health::check(&args);
    }

    let runtime = Runtime::new(&args)?;
    let server = Server::new(&args, &runtime)?;

    runtime.block_on(exec(&server))?;

    // Joins whatever the shutdown left running before a restart replaces the
    // process image.
    drop(runtime);

    #[cfg(unix)]
    if server.server.is_restarting() {
        restart::restart();
    }

    debug_info!("Exit");
    Ok(())
}

/// Starts, runs and stops the server within the asynchronous runtime.
#[tracing::instrument(
    name = "main",
    parent = None,
    skip_all
)]
async fn exec(server: &Arc<Server>) -> Result {
    let signals = server
        .server
        .runtime()
        .spawn(signal::enable(server.clone()));

    let abort = signals.abort_handle();
    defer! {{
        abort.abort();
    }}

    let started = start(server).await;

    // Services were never inserted, so there is nothing to run or stop, and the
    // listener must not open on a half-migrated database.
    if let Err(error) = &started
        && cancelled_by_shutdown(server, error)
    {
        signals.await?;
        return Ok(());
    }

    started?;
    run(server).await?;
    stop(server).await?;
    signals.await?;

    debug_info!("Exit runtime");
    Ok(())
}

/// Whether a failed startup is a stop request being honored rather than a
/// fault.
///
/// A cancelled startup exits zero, while a fault does not, and restart policies
/// keyed on failure act on the difference.
fn cancelled_by_shutdown(server: &Server, error: &Error) -> bool {
    error.is_interrupted() && server.server.is_stopping()
}

/// Builds the services and runs every startup phase, inserting them on success.
async fn start(server: &Arc<Server>) -> Result {
    match serve::start(server.server.clone()).await {
        Ok(services) => {
            server.services.lock().await.replace(services);
            Ok(())
        }
        Err(error) => {
            if cancelled_by_shutdown(server, &error) {
                info!("Stop requested during startup; exiting before the server began serving.");
            } else {
                error!("Critical error starting server: {error}");
            }

            Err(error)
        }
    }
}

async fn run(server: &Arc<Server>) -> Result {
    let services = server
        .services
        .lock()
        .await
        .clone()
        .expect("services initialized");

    serve::run(&services)
        .await
        .inspect_err(|error| error!("Critical error running server: {error}"))
}

async fn stop(server: &Arc<Server>) -> Result {
    let services = server
        .services
        .lock()
        .await
        .take()
        .expect("services initialized");

    serve::stop(services)
        .await
        .inspect_err(|error| error!("Critical error stopping server: {error}"))
}
