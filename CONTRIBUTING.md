# Contributing

## Layout

- `crates/` — Rust workspace (Matrix homeserver)
  - `phantom-core` — shared types: config, errors, logging, allocators, Matrix events, state resolution
  - `phantom-database` — RocksDB engine, typed columns, codecs, read pool
  - `phantom-service` — the service runtime and the services built on it
  - `phantom-macros` — proc macros, including the config-example generator
  - `phantom-server` — the binary (currently a stub)
- `cli/` — Go module (`phantom` admin CLI)
- `docs/` — everything below is expanded there

## Getting set up

[docs/installation.md](docs/installation.md) covers the toolchains and their
platform packages. In short: Rust 1.97.1 via `rustup` (pinned, so it selects
itself), Go 1.26+, and a C/C++ compiler with `libclang` for the bundled
RocksDB.

## Checks

Run `just check` before opening a pull request. It is exactly what CI runs:
`cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
`cargo test --workspace`, `go vet` and `go test -race`.

Clippy runs with `-D warnings`, so a warning fails the build. The workspace is
warning-free — keep it that way rather than leaving one for later.

## Conventions

**No catch-all modules.** There is no `utils`. A module is named for its
subject, and a module full of extension traits is named for the type it
extends. A helper that fits nowhere means the subject it belongs to has not
been named yet.

**Comment the reasoning.** The doc comments here explain why a thing is shaped
the way it is, not just what it does. When you change a decision, change the
comment that explains it.

**Config documentation lives on the struct field**, in
`crates/phantom-core/src/config/mod.rs`. `phantom-example.toml` is generated
from it on every real `cargo build` — commit the regenerated file alongside
your change, and never edit it directly.

**Divergences from conduwuit get a comment at the site**, saying what changed
and why. If it is a whole subsystem, add it to `NOTICE` too.

[docs/development.md](docs/development.md) has the rest, including the handful
of things about this workspace that will surprise you once.

## Security

Do not open a public issue for a vulnerability. See [SECURITY.md](SECURITY.md).
