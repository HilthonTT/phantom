# phantom

A [Matrix](https://matrix.org) homeserver written in Rust, with a terminal
admin console written in Go.

> **Status: early development.** There is no running server yet — the binary is
> a stub whose `main` is a `todo!()`. What exists is the foundation:
> configuration, error handling, logging, the Matrix event types and state
> resolution, the RocksDB storage layer, and the service runtime the server
> will be built out of. The admin console runs, but draws placeholder data.
> **Don't deploy this.**

## What's here

| | State |
| :--- | :--- |
| **`phantom-core`** — config, errors, logging, allocators, Matrix events and state resolution | usable |
| **`phantom-database`** — RocksDB engine, 88 typed columns, codecs, the read pool | usable |
| **`phantom-service`** — the service runtime and 30-odd services on it: rooms and their state, the timeline write path, the federation event handler, spaces, media, sync, sending, users, push | partial |
| **`phantom-macros`** — the config-example generator and friends | usable |
| **`phantom-server`** — the binary | a stub |
| **`cli/`** — the `phantom` admin console | runs on placeholder data |

[docs/architecture.md](docs/architecture.md) has the full picture.

## Getting started

You need **Rust 1.97.1** (pinned by `rust-toolchain.toml`, so `rustup` selects
it for you), **Go 1.26+** for the CLI, and a **C/C++ compiler with `libclang`**
— `phantom-database` compiles a bundled RocksDB. [`just`](https://github.com/casey/just)
is optional but every recipe below assumes it.

```sh
# Debian/Ubuntu; other platforms are in the install guide
sudo apt install -y build-essential clang libclang-dev pkg-config git curl
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

git clone https://github.com/HilthonTT/phantom.git
cd phantom
just build          # cargo build --release, then the Go CLI into target/phantom
./target/phantom    # the admin console
```

**[docs/installation.md](docs/installation.md)** is the full guide: packages for
Debian, Fedora, Arch, Alpine, macOS and Windows/WSL, optional build features,
and what to do when the build fails.

## Building and checking

```sh
just build          # release build of both halves
just check          # everything CI runs: fmt, clippy, and tests for both languages
```

`just check-rust` is `cargo fmt --check`, `cargo clippy --workspace
--all-targets -- -D warnings` and `cargo test --workspace`; `just check-go` is
`go vet` and `go test -race`. Clippy runs with `-D warnings`, so a warning
fails the build — the workspace is warning-free and is meant to stay that way.

Run `just check` before opening a pull request.

## Configuration

phantom reads TOML, layered in this order (later wins):

1. the file named by `$PHANTOM_CONFIG`
2. any config paths passed on the command line
3. environment variables prefixed `PHANTOM_`, where `__` separates nested keys —
   so `PHANTOM_SERVER_NAME` sets `server_name`

Every option lives under a `[global]` section:

```toml
[global]
server_name = "phantom.chat"
database_path = "/var/lib/phantom"
```

Unknown keys are collected rather than rejected, and logged as a warning at
startup so a typo is visible instead of silently ignored.

**`phantom-example.toml` is generated — don't edit it.** It is written to the
repository root on every real `cargo build`, derived from the `Config` struct by
the `#[config_example_generator]` proc macro: each field's doc comment becomes
that option's documentation. To document or add an option, edit
`crates/phantom-core/src/config/mod.rs`; changes made to the example file are
overwritten by the next build.

[docs/configuration.md](docs/configuration.md) covers the rest, including the
doc-comment directives (`default:`, `display: hidden`, `display: sensitive`) and
what validation rejects versus warns about.

## The admin console

`./target/phantom` opens a terminal interface modelled on
[superfile](https://github.com/yorukot/superfile): a section navigator down the
left, listings across the middle, a detail panel on the right, and a row of
boxes along the bottom for running tasks, the current selection and the
connection. Press `?` for the keys, `q` to quit.

Nothing behind it is real yet — it reads no config and opens no socket.
[docs/cli.md](docs/cli.md) describes it.

## Allocators

`phantom-core` builds against the system allocator by default. Alternatives are
Cargo features, all of them no-ops on MSVC targets where their C libraries do
not build:

| Feature | Effect |
| :--- | :--- |
| `jemalloc` | use jemalloc |
| `jemalloc_conf` | jemalloc with compile-time tuning |
| `jemalloc_prof` | jemalloc with heap profiling |
| `jemalloc_stats` | jemalloc with statistics collection |
| `hardened_malloc` | use hardened_malloc (ignored if `jemalloc` is also set) |

## Relationship to conduwuit

phantom began as, and still tracks, a port of
[conduwuit](https://github.com/girlbossceo/conduwuit). Substantial portions of
the codebase are derived from it, sometimes verbatim and sometimes adapted —
most visibly the configuration, error, macro and database layers.

Where phantom diverges, it is usually for one of two reasons: conduwuit pins an
older fork of [ruma](https://github.com/ruma/ruma) whose API has since moved on,
or a subsystem phantom hasn't ported yet has been trimmed rather than stubbed.
Divergences are commented at the site where they occur — see
[docs/upstream-sync.md](docs/upstream-sync.md).

conduwuit is licensed under the Apache License 2.0, which is why phantom is too.
Attribution and a summary of what has been changed are in [NOTICE](NOTICE).

## Documentation

| | |
| :--- | :--- |
| [installation.md](docs/installation.md) | toolchains, building, troubleshooting |
| [architecture.md](docs/architecture.md) | the crates and how they layer |
| [configuration.md](docs/configuration.md) | settings, and adding an option |
| [cli.md](docs/cli.md) | the admin console |
| [development.md](docs/development.md) | checks, CI, tests, conventions |
| [deployment.md](docs/deployment.md) | what's decided so far — you cannot deploy yet |
| [upstream-sync.md](docs/upstream-sync.md) | tracking conduwuit |

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md).

## Security

Report vulnerabilities privately using GitHub's
[report a vulnerability](https://github.com/HilthonTT/phantom/security/advisories/new)
form, not a public issue. See [SECURITY.md](SECURITY.md).

## License

Licensed under the Apache License, Version 2.0 — see [LICENSE](LICENSE).
Third-party attribution is in [NOTICE](NOTICE).
