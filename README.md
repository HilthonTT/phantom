# phantom

A [Matrix](https://matrix.org) homeserver written in Rust.

> **Status: early development.** There is no running server yet — the binary is
> a stub. What exists today is the foundation: configuration, error handling,
> allocator integration, and shared utilities. Don't deploy this.

## Requirements

- **Rust 1.97.1** — pinned by `rust-toolchain.toml`, so `rustup` selects it for you
- **Go 1.26** — for the `phantom` admin CLI
- **[`just`](https://github.com/casey/just)** — optional, but the recipes below assume it

## Building

```sh
just build          # cargo build --release, then the Go CLI into target/phantom
just check          # everything CI runs: fmt, clippy, and tests for both languages
```

The individual recipes are `check-rust` (`cargo fmt --check`, `cargo clippy
--workspace --all-targets -- -D warnings`, `cargo test --workspace`) and
`check-go` (`go vet`, `go test -race`). Run `just check` before opening a pull
request.

Clippy runs with `-D warnings`, so a warning fails the build. The workspace is
currently warning-free — please keep it that way.

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

### `phantom-example.toml` is generated — don't edit it

A documented example config is written to the repository root on every
`cargo build`. It is generated from the `Config` struct by the
`#[config_example_generator]` proc macro: each field's doc comment becomes that
option's documentation, and its `#[serde(default = "...")]` becomes the value
shown.

**To document or add a config option, edit `crates/phantom-core/src/config/mod.rs`.**
Changes made directly to `phantom-example.toml` are overwritten by the next
build.

Doc comments understand three directives on a line of their own:

| Directive | Effect |
| :--- | :--- |
| `default: <text>` | overrides the value shown in the example file |
| `display: hidden` | omits the option when a running server prints its config |
| `display: sensitive` | masks the value as `***********` instead of printing it |

## Allocators

`phantom-core` builds against the system allocator by default. Two alternatives
are available as Cargo features, both no-ops on MSVC targets, where their C
libraries do not build:

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
most visibly the configuration, error, and macro layers.

Where phantom diverges, it is usually for one of two reasons: conduwuit pins an
older fork of [ruma](https://github.com/ruma/ruma) whose API has since moved on,
or a subsystem phantom hasn't ported yet has been trimmed rather than stubbed.
Divergences are commented at the site where they occur.

conduwuit is licensed under the Apache License 2.0, which is why phantom is too.
Attribution and a summary of what has been changed are in [NOTICE](NOTICE).

## Documentation

Further documentation lives in [`docs/`](docs/) — currently a set of
placeholders.

## Security

Report vulnerabilities privately using GitHub's
[report a vulnerability](https://github.com/HilthonTT/phantom/security/advisories/new)
form, not a public issue. See [SECURITY.md](SECURITY.md).

## License

Licensed under the Apache License, Version 2.0 — see [LICENSE](LICENSE).
Third-party attribution is in [NOTICE](NOTICE).
