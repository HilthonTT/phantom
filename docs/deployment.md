# Deployment

**You cannot deploy phantom.** `crates/phantom-server`'s `main` is a `todo!()`:
the binary builds, starts, and panics. There is no HTTP surface, no federation,
and no way for a Matrix client to connect to it. Nothing on this page is a
recommendation to run one.

What follows is what the codebase already decides about how a deployed phantom
would work, so that the shape is on record and the eventual guide has somewhere
to grow from.

## What already exists

**Configuration is deployment-ready.** Settings layer from a file named by
`$PHANTOM_CONFIG`, then paths on the command line, then `PHANTOM_`-prefixed
environment variables — which is the arrangement container deployments want:
ship one file, override the handful of values that differ. See
[configuration.md](configuration.md).

**Config reload without a restart.** `config_reload_signal` (default on) makes
the server re-read its config on `SIGUSR1`. Only `server_name` is fixed for the
life of the process; a reload that changes it is refused.

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

- Any HTTP listener, and therefore any client or federation API
- A container image — `release.yml` has a placeholder job for
  `ghcr.io/hilthontt/phantom` and nothing behind it
- Release binaries — the CLI job is a placeholder waiting on a
  `.goreleaser.yaml`
- Reverse-proxy, TLS or `.well-known` guidance
- A `systemd` unit file
- Any migration path, since there is nothing yet to migrate from

## Following along

The gap between this page and a real deployment guide is the gap between the
service layer and a running server. [architecture.md](architecture.md)
describes what is built and what the binary would have to wire together.
