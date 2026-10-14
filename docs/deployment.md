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

An admin takes one with `!admin server backup` or the console's `backup`;
`database_backup_interval_s` takes them on a schedule as well:

```toml
[global]
database_backup_path = "/var/backups/phantom"
database_backups_to_keep = 7
database_backup_interval_s = 86400   # daily
```

The interval counts from the newest backup in the directory, not from
startup, so restarts do not postpone it; a server that has never backed up
takes its first one shortly after it starts. A failed backup is retried an
interval later. Each backup prunes the directory down to the newest
`database_backups_to_keep`. The console's overview, `!admin server status`
and the admin stats show the last backup and when the next is due. A backup
runs as an admin task, but one that cannot be cancelled: RocksDB gives no way
to interrupt it.

**Cancelling admin tasks.** Room shutdowns and deletions run as tasks
(`!admin tasks list`) and can be stopped with `!admin tasks cancel <id>`,
`POST /_phantom/admin/v1/tasks/{id}/cancel`, or the Tasks section's action.
A task stops at the next point where that leaves the room consistent: a
shutdown between two members' evictions, leaving the rest joined and the
aliases and directory listing untouched; a deletion likewise, or after the
shutdown but before the purge. Once the purge has started it runs to the end,
because a room with some of its tables purged and others not is neither intact
nor gone. Running the deletion again finishes a cancelled one. A stopped task
reads `cancelled`, with what it had done in its error.

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

**Prometheus metrics.** `allow_metrics` serves them at `/metrics`, in the
Prometheus text format. Off by default. Either protect them with a bearer
token on the main listeners, or serve them on a listener of their own, which
is plain HTTP and should be bound where only the scraper reaches it:

```toml
[global]
allow_metrics = true
metrics_token = "a-long-random-string"   # required on the main listeners
# metrics_address = "127.0.0.1:9090"     # or: a separate listener; token optional
```

The server refuses to start with metrics on, no token and no separate
address. A Prometheus scrape config to match:

```yaml
scrape_configs:
  - job_name: phantom
    scheme: https                 # http for metrics_address
    metrics_path: /metrics
    authorization:
      type: Bearer
      credentials: a-long-random-string
    static_configs:
      - targets: ["matrix.example.com:443"]
```

What is exposed:

| Metric | What |
| :--- | :--- |
| `phantom_http_requests_duration_seconds` | histogram by `route` (the route template, never the raw path), `method` and `status` class |
| `phantom_http_requests_in_flight` | requests being answered |
| `phantom_sync_long_polls` | `/sync` and `/events` requests parked waiting |
| `phantom_sending_queue_events` | outgoing events waiting, by `class` (federation, appservice, push) and `state` (queued, active) |
| `phantom_sending_transactions_total` | outgoing transactions by `class` and `outcome` (sent, failed) |
| `phantom_federation_transactions_received_total`, `phantom_federation_pdus_received_total`, `phantom_federation_edus_received_total` | inbound federation, PDUs by `outcome` |
| `phantom_database_*` | block cache use per cache, SST bytes per column, memtables, table readers, pending compaction bytes, running compactions, last and next backup |
| `phantom_tasks` | admin tasks of the last week, by `status` |
| `phantom_local_users`, `phantom_local_users_active`, `phantom_rooms` | counts, refreshed at most once a minute |
| `phantom_uptime_seconds`, `process_start_time_seconds`, `phantom_build_info` | the process |
| `tokio_*` | the runtime's workers, alive tasks, global queue depth, busy time and parks |

The account and room counts and the sending queues are read with a database
scan, so they are cached for a minute rather than taken on every scrape.
tokio's further scheduler metrics need `--cfg tokio_unstable` at build time and
are not exported.

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
