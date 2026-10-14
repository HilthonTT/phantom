//! Start, serve and stop: what tuwunel keeps in its router crate, cut down to
//! TCP listeners, with TLS when configured, without the tower layer stack.

use std::{
    net::SocketAddr,
    sync::{Arc, Weak},
    time::Duration,
};

use axum::{
    Extension, Json, Router,
    extract::DefaultBodyLimit,
    http::StatusCode,
    middleware,
    response::{IntoResponse, Response},
    serve::ListenerExt,
};
use phantom_api::router::{ConfiguredIpSource, Routes, State};
use phantom_core::{
    Err, Error, Result, debug, debug_error, debug_info, error, info,
    runtime::server::Server as CoreServer,
};
use phantom_service::Services;
use serde_json::json;
use tokio::{net::TcpListener, task::JoinSet, time::timeout};

use crate::tls::{Certs, TlsListener};

/// How long in-flight requests get to finish once shutdown begins; a sync
/// long-poll left running past this is cut off.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);

/// Builds the services and runs every startup phase.
#[tracing::instrument(skip_all)]
pub(crate) async fn start(server: Arc<CoreServer>) -> Result<Arc<Services>> {
    debug!("Starting...");

    let services = Services::build(server)?;

    // Before start, so the config's admin_execute commands find it.
    services
        .admin
        .set_processor(Some(phantom_api::admin::commands::processor));
    services
        .admin
        .set_completer(Some(phantom_api::admin::commands::complete));

    let services = services.start().await?;

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
    let certs = Certs::load(&server)?;

    let mut listeners = JoinSet::new();
    for addr in config.get_bind_addrs() {
        let listener = match TcpListener::bind(addr).await {
            Ok(listener) => listener,
            Err(e) => {
                error!(%addr, "Failed to bind listener: {e}");
                continue;
            }
        };

        let server = server.clone();
        let shutdown = async move { server.until_shutdown().await };
        let certs = certs.as_ref().filter(|_| config.tls.on_port(addr.port()));
        match certs {
            None => {
                info!(%addr, "Listening");
                let serving = axum::serve(listener, app.clone()).with_graceful_shutdown(shutdown);
                listeners.spawn(async move { serving.await.map_err(Error::from) });
            }
            Some(certs) => {
                // `tap_io` does nothing to the stream; it is there because axum
                // only derives a `SocketAddr` connect info for a plain
                // `TcpListener` or a tapped listener.
                let listener = TlsListener::new(listener, certs.clone())?.tap_io(|_| ());

                info!(%addr, "Listening with TLS");
                let serving = axum::serve(listener, app.clone()).with_graceful_shutdown(shutdown);
                listeners.spawn(async move { serving.await.map_err(Error::from) });
            }
        }
    }

    if listeners.is_empty() {
        return Err!("No listener could be bound; see the errors above.");
    }

    if config.logging.allow_metrics
        && let Some(addr) = config.logging.metrics_address
    {
        // Plain HTTP whatever `[global.tls]` says: the address is meant to be
        // one only the scraper reaches, and Prometheus scrapes plain HTTP by
        // default. Failing to bind it does not stop the server serving.
        match TcpListener::bind(addr).await {
            Err(e) => error!(%addr, "Failed to bind the metrics listener: {e}"),
            Ok(listener) => {
                info!(%addr, "Serving metrics");
                let app = phantom_api::metrics::register_own(Router::new())
                    .with_state(State::new(services.clone()));
                let server = server.clone();
                let shutdown = async move { server.until_shutdown().await };
                let serving = axum::serve(listener, app).with_graceful_shutdown(shutdown);
                listeners.spawn(async move { serving.await.map_err(Error::from) });
            }
        }
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

    let routes = Routes::new();
    let routes = phantom_api::client::register(routes, config);
    let routes = phantom_api::server::register(routes, config);
    let routes = phantom_api::oidc::register(routes);
    let routes = phantom_api::admin::register(routes);

    // `/metrics` stays out of the route table: it is not part of any Matrix
    // API, and when `metrics_address` is set it is served elsewhere anyway.
    let router = routes.into_router();
    let metrics = config.logging.allow_metrics;
    let router = match metrics && config.logging.metrics_address.is_none() {
        true => phantom_api::metrics::register(router),
        false => router,
    };

    let router = router
        .fallback(not_found)
        .layer(DefaultBodyLimit::max(config.network.max_request_size));

    // Last, so it wraps the rest and times the whole request. Off when
    // metrics are, so a server nobody scrapes does not keep the series.
    let router = match metrics {
        true => router.layer(middleware::from_fn_with_state(
            services.server.clone(),
            phantom_api::metrics::track,
        )),
        false => router,
    };

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
