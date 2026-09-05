# phantom

A [Matrix](https://matrix.org) homeserver written in Rust, with a terminal admin
console written in Go.

[![rust](https://github.com/HilthonTT/phantom/actions/workflows/rust.yml/badge.svg)](https://github.com/HilthonTT/phantom/actions/workflows/rust.yml)
[![go](https://github.com/HilthonTT/phantom/actions/workflows/go.yml/badge.svg)](https://github.com/HilthonTT/phantom/actions/workflows/go.yml)
[![audit](https://github.com/HilthonTT/phantom/actions/workflows/audit.yml/badge.svg)](https://github.com/HilthonTT/phantom/actions/workflows/audit.yml)
[![license](https://img.shields.io/badge/license-Apache--2.0-blue)](LICENSE)

> [!WARNING]
> **phantom does not run yet.** `phantom-server`'s `main` is a `todo!()`, so the
> binary compiles and then panics. There is no HTTP surface, no federation
> endpoint, and no way to point a Matrix client at it. The admin console starts,
> but draws placeholder data. **Do not deploy this.**

What does exist is the whole layer underneath a homeserver: configuration,
errors, logging, the Matrix event types and state resolution, a RocksDB storage
engine, and forty services on a runtime that supervises them — the timeline
write path, the federation event handler, room state, spaces, sync, media,
sending, users and push among them. Roughly 50,000 lines of Rust across five
crates, plus a 4,000-line Go console. The part still missing is the one that
ties them to a socket.

## Status

| Crate | What it is | State |
| :--- | :--- | :--- |
| **`phantom-core`** | config, errors, logging, allocators, Matrix event types and state resolution | usable |
| **`phantom-database`** | RocksDB engine, 89 typed columns, codecs, the read pool | usable |
| **`phantom-service`** | the service runtime and the 40 services on it | partial |
| **`phantom-macros`** | proc macros, including the config-example generator | usable |
| **`phantom-server`** | the binary | a stub |
| **`cli/`** | the `phantom` admin console | runs on placeholder data |

"Partial" means the services are written and compile, but nothing drives them:
no request ever reaches one. [docs/architecture.md](docs/architecture.md) has
the full picture of how they layer.

## Quick start

You need **Rust 1.97.1** — pinned by `rust-toolchain.toml`, so `rustup` selects
it for you — **Go 1.27+** for the console, and a **C/C++ compiler with
`libclang`**, because `phantom-database` compiles a bundled RocksDB. That last
one is why the first build takes a while.
[`just`](https://github.com/casey/just) is optional, but every recipe here
assumes it.

```sh
# Debian/Ubuntu; other platforms are in the installation guide
sudo apt install -y build-essential clang libclang-dev pkg-config git curl
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

git clone https://github.com/HilthonTT/phantom.git
cd phantom
just build          # cargo build --release, then the Go console into target/phantom
./target/phantom    # opens the admin console
```

`./target/release/phantom-server` also exists after that build. Running it
panics with `not yet implemented` — that is the current state of the project,
not a broken build.

[**docs/installation.md**](docs/installation.md) is the real guide: packages for
Debian, Fedora, Arch, Alpine, macOS and Windows/WSL, optional build features
(jemalloc, hardened_malloc, systemd), and what to do when the build fails.

## Building and checking

```sh
just build          # release build of both halves
just check          # everything CI runs, for both languages
```

`just check-rust` runs `cargo fmt --check`, `cargo clippy --workspace
--all-targets -- -D warnings` and `cargo test --workspace`. `just check-go` runs
`go vet` and `go test -race`. Clippy runs with `-D warnings`, so a single
warning fails the build; the workspace is warning-free and is meant to stay that
way. Run `just check` before opening a pull request.

[docs/development.md](docs/development.md) covers the tests, the CI jobs, the
conventions, and the handful of things that will surprise you once.

## Configuration

phantom reads TOML. Every option lives under a `[global]` section:

```toml
[global]
server_name = "phantom.chat"
database_path = "/var/lib/phantom"
```

Sources are layered, later winning: the file named by `$PHANTOM_CONFIG`, then
any config paths given on the command line, then environment variables prefixed
`PHANTOM_` (where `__` separates nested keys, so `PHANTOM_SERVER_NAME` sets
`server_name`). Unknown keys are collected and logged as a warning at startup
rather than rejected, so a typo is visible instead of silent.

> **`phantom-example.toml` is generated — don't edit it.** Every real `cargo
> build` rewrites it from the `Config` struct via the
> `#[config_example_generator]` proc macro, turning each field's doc comment
> into that option's documentation. To add or document an option, edit
> `crates/phantom-core/src/config/mod.rs`.

[docs/configuration.md](docs/configuration.md) covers the doc-comment directives
(`default:`, `display: hidden`, `display: sensitive`) and what validation
rejects versus merely warns about.

## The admin console

`./target/phantom` opens a terminal interface modelled on
[superfile](https://github.com/yorukot/superfile): a section navigator down the
left, listings across the middle, a detail panel on the right, and a row of
boxes along the bottom for running tasks, the current selection and the
connection. Press `?` for the keys and `q` to quit.

Nothing behind it is real yet — it reads no config and opens no socket.
[docs/cli.md](docs/cli.md) describes the layout, the keys and the command
prompt.

## Relationship to conduwuit

phantom began as, and still tracks, a port of
[conduwuit](https://github.com/girlbossceo/conduwuit). Substantial portions of
the codebase are derived from it, sometimes verbatim and sometimes adapted —
most visibly the configuration, error, macro and database layers.

Where phantom diverges it is usually for one of two reasons: conduwuit pins an
older fork of [ruma](https://github.com/ruma/ruma) whose API has since moved on,
or a subsystem phantom hasn't ported yet has been trimmed rather than stubbed.
Divergences are commented at the site where they occur, and
[docs/upstream-sync.md](docs/upstream-sync.md) explains how the tracking works.

conduwuit is licensed under the Apache License 2.0, which is why phantom is too.
Attribution and a summary of what changed are in [NOTICE](NOTICE).

## Documentation

| Page | What it covers |
| :--- | :--- |
| [installation.md](docs/installation.md) | toolchains per platform, building, optional features, troubleshooting |
| [architecture.md](docs/architecture.md) | the crates, how they layer, and the service runtime |
| [configuration.md](docs/configuration.md) | where settings come from, and adding an option |
| [cli.md](docs/cli.md) | the admin console |
| [development.md](docs/development.md) | checks, CI, tests, conventions |
| [deployment.md](docs/deployment.md) | what the codebase already decides — you cannot deploy yet |
| [upstream-sync.md](docs/upstream-sync.md) | tracking conduwuit |

## Contributing

Issues and pull requests are welcome; [CONTRIBUTING.md](CONTRIBUTING.md) has the
expectations. Given the state of the project, the most useful contributions are
in the unported subsystems rather than in polish.

## Security

Report vulnerabilities privately through GitHub's
[report a vulnerability](https://github.com/HilthonTT/phantom/security/advisories/new)
form, not a public issue. See [SECURITY.md](SECURITY.md).

## License

Licensed under the Apache License, Version 2.0 — see [LICENSE](LICENSE).
Third-party attribution is in [NOTICE](NOTICE).
