# phantom

A [Matrix](https://matrix.org) homeserver written in Rust, with a terminal admin
console written in Go.

[![rust](https://github.com/HilthonTT/phantom/actions/workflows/rust.yml/badge.svg)](https://github.com/HilthonTT/phantom/actions/workflows/rust.yml)
[![go](https://github.com/HilthonTT/phantom/actions/workflows/go.yml/badge.svg)](https://github.com/HilthonTT/phantom/actions/workflows/go.yml)
[![audit](https://github.com/HilthonTT/phantom/actions/workflows/audit.yml/badge.svg)](https://github.com/HilthonTT/phantom/actions/workflows/audit.yml)
[![license](https://img.shields.io/badge/license-Apache--2.0-blue)](LICENSE)

> [!WARNING]
> **Early and not production-ready.** phantom serves the client-server API over
> plain HTTP, so local accounts can register, chat, sync and be managed from the
> console. Federation with other servers is mounted but untested, there is no
> TLS (put it behind a reverse proxy), room version 12 is unsupported, and the
> `!admin` room commands do nothing yet. **Do not deploy this.**

## Status

| Part | What it is | State |
| :--- | :--- | :--- |
| **`phantom-core`** | config, errors, logging, Matrix event types, state resolution | usable |
| **`phantom-database`** | RocksDB engine, 113 typed columns, codecs, read pool | usable |
| **`phantom-service`** | the service runtime and its ~55 services | usable, partly untested |
| **`phantom-api`** | client-server, federation, OIDC and admin HTTP routes | client API works |
| **`phantom-macros`** | proc macros, including the config-example generator | usable |
| **`phantom-server`** | the `phantom-server` binary | serves plain TCP |
| **`cli/`** | the `phantom` admin console | live for chat and most admin sections |

About 94,000 lines of Rust and 8,000 of Go.
[docs/architecture.md](docs/architecture.md) shows how the crates layer.

## Quick start

You need **Rust 1.97.1** (pinned by `rust-toolchain.toml`), **Go 1.27+**, a
**C/C++ compiler with `libclang`** for the bundled RocksDB, which makes the
first build slow, and optionally [`just`](https://github.com/casey/just).
[docs/installation.md](docs/installation.md) has the packages per platform.

```sh
git clone https://github.com/HilthonTT/phantom.git && cd phantom
just build      # release server, plus the console at target/phantom
```

A minimal `phantom.toml`:

```toml
[global]
server_name = "phantom.test"
database_path = "/var/lib/phantom"
allow_registration = true
registration_token = "change-me"
```

```sh
./target/release/phantom-server -c phantom.toml   # listens on localhost:8008
./target/phantom                                  # the console; --server URL to point elsewhere
```

A fresh database creates the `#admins` room. **The first account to register
becomes the server's admin.** Register it with any Matrix client and the token
above, then sign in to the console with it.

## The admin console

A terminal interface modelled on [superfile](https://github.com/yorukot/superfile):
sections on the left, a table in the middle, details on the right, and tasks,
the selection and the connection along the bottom. `?` lists the keys, `:`
opens the command prompt, `q` quits.

- **Chat** works with any account: rooms and messages over `/sync`, sending,
  typing notices, read receipts, `:join` and `:leave`. Encrypted rooms show
  their messages as undecryptable.
- **Overview, Users, Devices, Tokens, Rooms, Appservices and Settings** come
  from the admin API (`/_phantom/admin/v1`) when you sign in as an admin.
- **Services, Federation, Media, Tasks, Reports and Logs** still show sample
  data, and every table that isn't live says so in its footer.

Sessions are kept per server in `phantom/sessions.json` under your config
directory (`~/.config` on Linux), readable only by you. [docs/cli.md](docs/cli.md) has more on the layout and keys.

## Configuration

phantom reads TOML, with every option under `[global]`. Sources are layered,
later winning: the file in `$PHANTOM_CONFIG`, then `-c` paths, then `PHANTOM_`
environment variables (`__` separates nested keys). Unknown keys are logged as a
warning, not rejected.

`phantom-example.toml` documents every option. **It is generated, so don't edit
it:** each `cargo build` rewrites it from the doc comments on the `Config`
structs in `crates/phantom-core/src/runtime/config/`. See
[docs/configuration.md](docs/configuration.md).

## Development

```sh
just check      # what CI runs: fmt, clippy -D warnings and tests (with and
                # without all features), go vet and go test -race
```

The workspace is warning-free and should stay that way. See
[docs/development.md](docs/development.md) for tests, CI and conventions.

## Documentation

| Page | What it covers |
| :--- | :--- |
| [installation.md](docs/installation.md) | toolchains per platform, build features, troubleshooting |
| [architecture.md](docs/architecture.md) | the crates and the service runtime |
| [configuration.md](docs/configuration.md) | settings sources and adding an option |
| [cli.md](docs/cli.md) | the admin console |
| [development.md](docs/development.md) | checks, CI, tests, conventions |
| [deployment.md](docs/deployment.md) | what deployment will need |
| [upstream-sync.md](docs/upstream-sync.md) | tracking conduwuit |

## Upstream, contributing, security, license

phantom began as, and still tracks, a port of
[conduwuit](https://github.com/girlbossceo/conduwuit); large parts are derived
from it, which is why phantom is also Apache-2.0. Divergences are commented
where they occur. Attribution is in [NOTICE](NOTICE).

Issues and pull requests are welcome; see [CONTRIBUTING.md](CONTRIBUTING.md).
Report vulnerabilities privately through
[GitHub's form](https://github.com/HilthonTT/phantom/security/advisories/new),
not a public issue ([SECURITY.md](SECURITY.md)).

Licensed under the [Apache License 2.0](LICENSE).
