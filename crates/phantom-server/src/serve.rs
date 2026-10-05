//! Start, serve and stop: what tuwunel keeps in its router crate, cut down to
//! plain TCP listeners without the tower layer stack.

use std::{
    net::SocketAddr,
    sync::{Arc, Weak},
    time::Duration,
};

use axum::{
    Extension, Json, Router,
    extract::DefaultBodyLimit,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use phantom_api::router::{ConfiguredIpSource, State};
use phantom_core::{
    Err, Error, Result, debug, debug_error, debug_info, error, info,
    runtime::server::Server as CoreServer,
};
use phantom_service::Services;
use serde_json::json;
use tokio::{net::TcpListener, task::JoinSet, time::timeout};

/// How long in-flight requests get to finish once shutdown begins; a sync
/// long-poll left running past this is cut off.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);

/// Builds the services and runs every startup phase.
#[tracing::instrument(skip_all)]
pub(crate) async fn start(server: Arc<CoreServer>) -> Result<Arc<Services>> {
    debug!("Starting...");

    let services = Services::build(server)?.start().await?;

    debug!("Started");
    Ok(services)
}

/// Serves clients until shutdown, or until a service worker finishes, which
/// is fatal and takes the server down with it.
#[tracing::instrument(skip_all)]
pub(crate) async fn run(services: &Arc<Services>) -> Result {
    let server = &services.server;
    debug!("Running");

    let listener = server.runtime().spawn(serve(services.clone()));

    tokio::select! {
        res = listener => res.map_err(Error::from).unwrap_or_else(Err),
        res = services.poll() => {
            debug!("Service manager finished: {res:?}");
            if server.running()
                && let Err(e) = server.shutdown()
            {
                error!("Failed to send shutdown signal: {e}");
            }

            res
        },
    }
}

/// Stops the services and checks nothing still holds them.
#[tracing::instrument(skip_all)]
pub(crate) async fn stop(services: Arc<Services>) -> Result {
    debug!("Shutting down...");

    services.stop().await;

    // The complex of Arcs across the services can easily leave a reference held
    // somewhere improperly, and that hangs the database's close.
    debug!("Cleaning up...");
    let db = Arc::downgrade(&services.db);
    if let Err(services) = Arc::try_unwrap(services) {
        debug_error!(
            "{} dangling references to Services after shutdown",
            Arc::strong_count(&services)
        );
    }

    if Weak::strong_count(&db) > 0 {
        debug_error!(
            "{} dangling references to Database after shutdown",
            Weak::strong_count(&db)
        );
    }

    info!("Shutdown complete.");
    Ok(())
}

/// Binds every configured address and serves the API on each until shutdown.
///
/// An address that fails to bind is logged and skipped; only when none bind
/// does serving fail.
async fn serve(services: Arc<Services>) -> Result {
    let server = services.server.clone();
    let config = &server.config;

    let app = router(&services).into_make_service_with_connect_info::<SocketAddr>();

    let mut listeners = JoinSet::new();
    for addr in config.get_bind_addrs() {
        let listener = match TcpListener::bind(addr).await {
            Ok(listener) => listener,
            Err(e) => {
                error!(%addr, "Failed to bind listener: {e}");
                continue;
            }
        };

        info!(%addr, "Listening");
        let server = server.clone();
        let serving = axum::serve(listener, app.clone())
            .with_graceful_shutdown(async move { server.until_shutdown().await });

        listeners.spawn(async move { serving.await.map_err(Error::from) });
    }

    if listeners.is_empty() {
        return Err!("No listener could be bound; see the errors above.");
    }

    server.until_shutdown().await;
    debug!(timeout = ?SHUTDOWN_TIMEOUT, "Waiting for requests to finish...");

    let drained = timeout(SHUTDOWN_TIMEOUT, async {
        while let Some(res) = listeners.join_next().await {
            if let Err(e) = res.map_err(Error::from).and_then(|res| res) {
                error!("Listener finished with error: {e}");
            }
        }
    })
    .await;

    if drained.is_err() {
        info!("Requests still in flight after {SHUTDOWN_TIMEOUT:?}; closing them.");
        listeners.abort_all();
    }

    debug_info!("Stopped listening");
    Ok(())
}

fn router(services: &Arc<Services>) -> Router {
    let config = &services.server.config;

    let router = Router::new();
    let router = phantom_api::client::register(router, config);
    let router = phantom_api::server::register(router, config);
    let router = phantom_api::oidc::register(router);
    let router = phantom_api::admin::register(router);

    let router = router
        .fallback(not_found)
        .layer(DefaultBodyLimit::max(config.network.max_request_size));

    let router = match config.ip_source {
        Some(source) => router.layer(Extension(ConfiguredIpSource(source))),
        None => router,
    };

    router.with_state(State::new(services.clone()))
}

/// Matrix clients expect an unknown endpoint to answer `M_UNRECOGNIZED`
/// rather than an empty 404.
async fn not_found() -> Response {
    let body = json!({
        "errcode": "M_UNRECOGNIZED",
        "error": "Unrecognized request",
    });

    (StatusCode::NOT_FOUND, Json(body)).into_response()
}
