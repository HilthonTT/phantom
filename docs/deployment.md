# Deployment

phantom runs: `phantom-server` binds its listeners and serves the client-server
and federation APIs. What it lacks is packaging — there is no container image,
release binary or service unit yet (see [What does not exist](#what-does-not-exist)),
so today a deployment means building from source. See
[installation.md](installation.md).

This page records what the codebase already decides about running phantom in
production, so the eventual guide has somewhere to grow from.

## What already exists

**Configuration is deployment-ready.** Settings layer from a file named by
`$PHANTOM_CONFIG`, then paths on the command line, then `PHANTOM_`-prefixed
environment variables — which is the arrangement container deployments want:
ship one file, override the handful of values that differ. See
[configuration.md](configuration.md).

**TLS, directly or behind a proxy.** By default the listeners speak plain HTTP,
for a reverse proxy that terminates TLS in front of them. Setting both
`certs` and `key` under `[global.tls]` makes every listener serve HTTPS
instead, HTTP/2 included:

```toml
[global]
tls = { certs = "/etc/letsencrypt/live/example.com/fullchain.pem", key = "/etc/letsencrypt/live/example.com/privkey.pem" }
```

`ports` under `[global.tls]` narrows TLS to the ports it lists, and the other
ports in `port` stay plain HTTP: a federation port peers reach directly next to
a client port behind a proxy, say, or the plain 8008 and TLS 8448 that Matrix's
Complement suite expects:

```toml
[global]
port = [8008, 8448]
tls = { certs = "...", key = "...", ports = [8448] }
```

There is no serving both on one port.

**Config and certificate reload without a restart.** `config_reload_signal`
(default on) makes the server re-read its config on `SIGUSR1`. Only
`server_name` is fixed for the life of the process; a reload that changes it is
refused. The same signal makes the listeners re-read the TLS certificate and
key from the paths they started with, whatever `config_reload_signal` says, so a
renewal hook only has to send `SIGUSR1`. A certificate that fails to load is
logged and the old one stays in use.

**Health checks.** `phantom-server --health-check`, given the same config as the
running server, probes its listeners over loopback and exits zero when one
answers — over TLS when `[global.tls]` is set. It checks liveness only, so it
does not verify the certificate.

**`.well-known` delegation.** `well_known_client` and `well_known_server` fill
`/.well-known/matrix/client` and `/.well-known/matrix/server`, and the
`well_known_support_*` options fill `/.well-known/matrix/support`.

**systemd integration.** Building `phantom-service` with its `systemd` feature
makes the manager notify systemd that it is reloading and, afterwards, that it
is ready again, so a `Type=notify` unit does not treat a reload as finished
before it is.

**Online database backups.** `database_backup_path` turns them on, and
`database_backups_to_keep` bounds how many are retained — a negative value
keeps every one. They go through RocksDB's backup engine, so the server does
not have to be stopped to take one. Leaving the path unset disables backups.

**Read-only and secondary modes.** `rocksdb_read_only` and `rocksdb_secondary`
exist for opening a database without taking write ownership of it.

**Storage-aware tuning.** The database read pool is sized and laid out after
the storage device rather than the CPU, because what a cache miss waits on is
the device's queue depth. `db_pool_workers`, `db_pool_workers_limit`,
`db_pool_queue_mult` and `db_pool_affinity` are the knobs;
`rocksdb_optimize_for_spinning_disks` and `rocksdb_direct_io` are the ones that
matter most on the storage side.

**Allocator choice at build time.** jemalloc, with optional profiling and
statistics, or hardened_malloc. See
[installation.md](installation.md#allocators).

**Metrics.** `allow_metrics` gates them; `phantom-core::metrics` reads tokio's
scheduler metrics, which need `--cfg tokio_unstable` at build time.

## What does not exist

- A container image — `release.yml` has a placeholder job for
  `ghcr.io/hilthontt/phantom` and nothing behind it
- Release binaries — the CLI job is a placeholder waiting on a
  `.goreleaser.yaml`
- A worked reverse-proxy or delegation guide — the options above exist, but no
  example nginx, Caddy or DNS setup does
- A `systemd` unit file
- An import path from another homeserver

## Following along

[configuration.md](configuration.md) covers every option named here, and
[architecture.md](architecture.md) describes how the server is put together.
