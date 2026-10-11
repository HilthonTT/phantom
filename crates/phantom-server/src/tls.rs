//! TLS termination for the listeners, when `[global.tls]` is configured.
//!
//! Handshakes run on their own tasks so a slow or stalled client never holds
//! up the accept loop; `axum::serve` only ever sees finished TLS streams.

use std::{
    io,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{Arc, PoisonError, RwLock},
    time::Duration,
};

use axum::serve::Listener;
use phantom_core::{Result, debug, err, error, info, runtime::server::Server as CoreServer, warn};
use rustls::{
    ServerConfig,
    crypto::ring,
    pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject},
};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::{broadcast::error::RecvError, mpsc},
    time::{sleep, timeout},
};
use tokio_rustls::{TlsAcceptor, server::TlsStream};

/// How long a client gets to finish the handshake before it is dropped.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// Finished handshakes waiting for `axum::serve` to pick them up.
const ACCEPT_QUEUE: usize = 128;

/// The signal that re-reads the certificate files, alongside the config.
const RELOAD_SIGNAL: &str = "SIGUSR1";

/// The server's certificate, shared by every listener and swapped in place
/// when the files are reloaded.
#[derive(Clone)]
pub(crate) struct Certs {
    config: Arc<RwLock<Arc<ServerConfig>>>,
}

impl Certs {
    /// Loads the configured certificate, or `None` when TLS is off.
    pub(crate) fn load(server: &Arc<CoreServer>) -> Result<Option<Self>> {
        let Some((certs, key)) = server.config.tls.paths() else {
            return Ok(None);
        };

        let config = server_config(certs, key)?;
        info!(certs = %certs.display(), "Loaded TLS certificate");

        let this = Self {
            config: Arc::new(RwLock::new(Arc::new(config))),
        };

        server.runtime().spawn(this.clone().reload_on_signal(
            server.clone(),
            certs.clone(),
            key.clone(),
        ));

        Ok(Some(this))
    }

    fn acceptor(&self) -> TlsAcceptor {
        let config = self.config.read().unwrap_or_else(PoisonError::into_inner);

        TlsAcceptor::from(Arc::clone(&config))
    }

    /// Re-reads the files on every reload signal until shutdown. A file that
    /// fails to load leaves the current certificate in place.
    async fn reload_on_signal(self, server: Arc<CoreServer>, certs: PathBuf, key: PathBuf) {
        let mut signals = server.signal.subscribe();
        while server.running() {
            match signals.recv().await {
                Ok(RELOAD_SIGNAL) => {}
                Ok(_) | Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => break,
            }

            match server_config(&certs, &key) {
                Ok(config) => {
                    *self.config.write().unwrap_or_else(PoisonError::into_inner) = Arc::new(config);
                    info!(certs = %certs.display(), "Reloaded TLS certificate");
                }
                Err(e) => error!("Failed to reload TLS certificate, keeping the old one: {e}"),
            }
        }
    }
}

fn server_config(certs: &Path, key: &Path) -> Result<ServerConfig> {
    let chain = CertificateDer::pem_file_iter(certs)
        .and_then(Iterator::collect::<Result<Vec<_>, _>>)
        .map_err(|e| err!(Config("tls.certs", "{}: {e}", certs.display())))?;

    if chain.is_empty() {
        return Err(err!(Config(
            "tls.certs",
            "{} holds no certificates",
            certs.display()
        )));
    }

    let key = PrivateKeyDer::from_pem_file(key)
        .map_err(|e| err!(Config("tls.key", "{}: {e}", key.display())))?;

    // Both ring and aws-lc-rs end up compiled into rustls through other
    // crates, and with two providers present rustls will not pick one itself.
    let mut config = ServerConfig::builder_with_provider(Arc::new(ring::default_provider()))
        .with_safe_default_protocol_versions()
        .and_then(|builder| builder.with_no_client_auth().with_single_cert(chain, key))
        .map_err(|e| err!(Config("tls", "{e}")))?;

    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];

    Ok(config)
}

/// A TCP listener that hands `axum::serve` connections with the TLS
/// handshake already done.
pub(crate) struct TlsListener {
    conns: mpsc::Receiver<(TlsStream<TcpStream>, SocketAddr)>,
    local_addr: SocketAddr,
}

impl TlsListener {
    pub(crate) fn new(tcp: TcpListener, certs: Certs) -> io::Result<Self> {
        let local_addr = tcp.local_addr()?;
        let (tx, conns) = mpsc::channel(ACCEPT_QUEUE);
        tokio::spawn(accept_loop(tcp, certs, tx));

        Ok(Self { conns, local_addr })
    }
}

impl Listener for TlsListener {
    type Addr = SocketAddr;
    type Io = TlsStream<TcpStream>;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        match self.conns.recv().await {
            Some(conn) => conn,
            // The accept loop only ends once this receiver is gone.
            None => std::future::pending().await,
        }
    }

    fn local_addr(&self) -> io::Result<Self::Addr> {
        Ok(self.local_addr)
    }
}

/// Accepts TCP connections and spawns a handshake for each, until the
/// listener is dropped; dropping the TCP listener then frees the port.
async fn accept_loop(
    tcp: TcpListener,
    certs: Certs,
    tx: mpsc::Sender<(TlsStream<TcpStream>, SocketAddr)>,
) {
    loop {
        let (stream, addr) = tokio::select! {
            () = tx.closed() => break,
            res = tcp.accept() => match res {
                Ok(conn) => conn,
                Err(e) => {
                    handle_accept_error(e).await;
                    continue;
                },
            },
        };

        let acceptor = certs.acceptor();
        let tx = tx.clone();
        tokio::spawn(async move {
            match timeout(HANDSHAKE_TIMEOUT, acceptor.accept(stream)).await {
                Ok(Ok(tls)) => _ = tx.send((tls, addr)).await,
                Ok(Err(e)) => debug!(%addr, "TLS handshake failed: {e}"),
                Err(_) => debug!(%addr, "TLS handshake timed out"),
            }
        });
    }
}

/// Mirrors axum's own listener: errors tied to one connection are dropped,
/// anything else (such as running out of file descriptors) backs off first.
async fn handle_accept_error(e: io::Error) {
    use io::ErrorKind::{ConnectionAborted, ConnectionRefused, ConnectionReset};

    if matches!(
        e.kind(),
        ConnectionRefused | ConnectionAborted | ConnectionReset
    ) {
        return;
    }

    warn!("Failed to accept connection: {e}");
    sleep(Duration::from_secs(1)).await;
}
