//! Liveness probe of a running server for `--health-check`.

use std::{
    io::{Read, Write},
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream},
    time::Duration,
};

use phantom_core::{Err, Result};

use crate::{args::Args, server::load_config};

const REQUEST: &[u8] =
    b"GET /_phantom/server_version HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n";

const TIMEOUT: Duration = Duration::from_secs(5);

/// Probe the listeners of a running server sharing this configuration,
/// exiting zero when one answers.
pub(crate) fn check(args: &Args) -> Result {
    // Built the same way the running server built its own, so the probe reads
    // the listeners it actually opened.
    let config = load_config(args)?;

    // The server keeps serving when one listener fails to bind, so healthy means
    // any listener answering.
    let mut last = Err!("No listeners are configured.");
    for addr in config.get_bind_addrs().into_iter().map(loopback) {
        last = probe_tcp(addr);
        if last.is_ok() {
            break;
        }
    }

    last
}

fn probe<S: Read + Write>(mut stream: S) -> Result {
    stream.write_all(REQUEST)?;

    let mut head = [0_u8; 12];
    stream.read_exact(&mut head)?;

    if head.starts_with(b"HTTP/1.") && head.ends_with(b" 200") {
        return Ok(());
    }

    let head = String::from_utf8_lossy(&head);

    Err!("Unexpected response from listener: {head:?}")
}

fn loopback(addr: SocketAddr) -> SocketAddr {
    match addr.ip() {
        IpAddr::V4(ip) if ip.is_unspecified() => {
            SocketAddr::new(Ipv4Addr::LOCALHOST.into(), addr.port())
        }
        IpAddr::V6(ip) if ip.is_unspecified() => {
            SocketAddr::new(Ipv6Addr::LOCALHOST.into(), addr.port())
        }
        _ => addr,
    }
}

fn probe_tcp(addr: SocketAddr) -> Result {
    let stream = TcpStream::connect_timeout(&addr, TIMEOUT)?;

    stream.set_read_timeout(Some(TIMEOUT))?;
    stream.set_write_timeout(Some(TIMEOUT))?;

    probe(stream)
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Cursor, Read, Write},
        net::SocketAddr,
    };

    use super::{loopback, probe};

    /// Replays a canned response and swallows whatever is written.
    struct Canned(Cursor<&'static [u8]>);

    impl Read for Canned {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.0.read(buf)
        }
    }

    impl Write for Canned {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn a_200_is_healthy() {
        probe(Canned(Cursor::new(b"HTTP/1.1 200 OK\r\n\r\n"))).expect("healthy");
    }

    #[test]
    fn anything_else_is_not() {
        probe(Canned(Cursor::new(b"HTTP/1.1 404 Not Found\r\n\r\n"))).expect_err("unhealthy");
        probe(Canned(Cursor::new(b"HTTP/1"))).expect_err("truncated");
    }

    #[test]
    fn unspecified_addresses_probe_loopback() {
        let v4: SocketAddr = "0.0.0.0:8008".parse().expect("valid");
        let v6: SocketAddr = "[::]:8008".parse().expect("valid");
        let named: SocketAddr = "10.0.0.1:8008".parse().expect("valid");

        assert_eq!(loopback(v4), "127.0.0.1:8008".parse().expect("valid"));
        assert_eq!(loopback(v6), "[::1]:8008".parse().expect("valid"));
        assert_eq!(loopback(named), named);
    }
}
