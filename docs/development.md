# Development

Working on phantom: the checks, the layout conventions, and the things about
this workspace that will surprise you once.

Set your machine up first — [installation.md](installation.md) covers the
toolchains and their platform packages.

## The checks

One command runs everything CI runs:

```sh
just check
```

Which is:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cd cli && go vet ./... && go test -race ./...
```

`just check-rust` and `just check-go` run one half each.

**Clippy runs with `-D warnings`.** A warning fails the build. The workspace is
warning-free today; keep it that way rather than leaving one for later.

## What CI does

| Workflow | Trigger | Runs |
| :--- | :--- | :--- |
| `rust.yml` | pushes to `main`, PRs touching `crates/`, `Cargo.*`, `rust-toolchain.toml` | `cargo fmt --check`, `clippy -D warnings`, `cargo test --workspace` |
| `go.yml` | pushes to `main`, PRs touching `cli/` or `.golangci.yml` | `go vet`, `golangci-lint`, `go test -race` |
| `audit.yml` | weekly, and on demand | `cargo-deny`, `govulncheck` |
| `release.yml` | tags matching `v*` | placeholder — the CLI release and the server image are both still TODO |

The Rust job has a 45-minute timeout, because a deadlocked test would otherwise
sit there until the job's own six-hour limit.

`cargo-deny` enforces the licence allowlist in `deny.toml`: Apache-2.0, MIT,
BSD-2-Clause, BSD-3-Clause, ISC and Unicode-3.0. A dependency under anything
else fails the audit.

## Tests

```sh
cargo test --workspace                    # everything
cargo test -p phantom-database            # one crate
cargo test -p phantom-core state_res      # one area
```

Where the interesting ones live:

- `phantom-core/src/matrix/pdu/tests.rs` and `matrix/state_res/` — state
  resolution against fixtures dense enough that `maplit`'s `hashmap!`/`hashset!`
  literals read better than chained inserts.
- `phantom-database/src/tests.rs` and the map tests — these open a real
  database under a `tempfile` directory, and build a `Config` from a TOML string
  the same way the server does.
- `phantom-service/src/resolver/tests.rs` — the server-name resolution steps.
- `phantom-core/tests/alloc_je.rs` — only meaningful with a jemalloc feature on.
- `cli/internal/tui/{app,panel}/*_test.go` — rendering and truncation.

The `#[bench]` suites in `matrix/state_res/benches.rs` are gated behind
`--cfg phantom_bench` and need a nightly compiler:

```sh
RUSTFLAGS="--cfg phantom_bench" cargo +nightly bench -p phantom-core
```

## Conventions

**No catch-all modules.** There is no `utils`. A module is named for its
subject, and a module full of extension traits is named for the type it
extends — `result`, `future`, `stream`, `bool`. A helper that fits nowhere is
treated as a sign the subject it belongs to has not been named yet. This holds
in the Go tree too.

**Comment the reasoning, not the mechanics.** The module docs in this workspace
explain why a thing is shaped the way it is. When you change a decision, change
the comment that explains it.

**Divergences from upstream get a comment where they occur.** See
[upstream-sync.md](upstream-sync.md).

**Config documentation lives on the struct field.** Never in
`phantom-example.toml`, which is generated — see
[configuration.md](configuration.md).

## Things that will surprise you once

**`phantom-example.toml` changes when you build.** It is regenerated from the
`Config` struct on every real `cargo build`. If it shows up dirty in `git
status` after you touched the config module, that is correct — commit it. If it
shows up dirty when you did not, someone else's change to the struct was
committed without the regenerated file.

**`cargo check` does not regenerate it.** The generator inspects rustc's own
command line and only writes when rustc is actually linking, so `cargo check`
and rust-analyzer skip it — otherwise the editor would rewrite the file on
every keystroke.

**Two files under `alloc/` are never both compiled.** `alloc::je` and
`alloc::hardened` each define a `#[global_allocator]`, so `hardened` is gated
on `not(feature = "jemalloc")`. rust-analyzer resolves one `cfg` set at a time,
so whichever is not selected reports "not included in any crates". To work on
the hardened path, change `rust-analyzer.cargo.features` in
`.vscode/settings.json` to `["hardened_malloc"]` and restart the server.
`--all-features` does *not* give you both: it enables `jemalloc`, which
excludes the other.

**`cfg(disable)` is never set, deliberately.** It parks an alternative
definition next to the one in use without compiling it. It is declared in
`Cargo.toml`'s `check-cfg` list along with `tokio_unstable`, `unabridged` and
`phantom_bench`, so using one does not warn.

**`axum` is a dependency of `phantom-core`.** Not because core serves HTTP, but
because `impl IntoResponse for Error` must live in the crate that defines
`Error`, and the orphan rule leaves it no other home.

**The first build takes a while.** `phantom-database` compiles a bundled
RocksDB. See [installation.md](installation.md#disk-and-time).

## Editor setup

`.vscode/settings.json` is checked in and is worth reading before you change
it. The notable settings:

- `rust-analyzer.check.command: "clippy"` — so editor diagnostics are the ones
  CI enforces. Do **not** add `--all-targets` to `check.extraArgs`;
  rust-analyzer already passes it, and cargo rejects the flag twice with
  "cannot be used multiple times", which kills flycheck.
- `rust-analyzer.cargo.targetDir: true` — keeps rust-analyzer out of `./target`,
  so its checks do not take the same cargo lock as a terminal build.
- `files.eol: "\n"` — the tree is LF, and is edited from both Windows and WSL.

On Windows, open the folder through the WSL remote so rust-analyzer runs
against the Linux toolchain.

## Pull requests

Run `just check` before opening one. Beyond that:

- Keep `phantom-example.toml` in step with the config struct in the same
  commit.
- If you diverge from conduwuit, say why in a comment at the site, and note it
  in `NOTICE` if it is a substantial subsystem.
- The workspace lints `unsafe_code = "warn"`. What `unsafe` there is sits where
  the workspace touches C: the allocator integration, the `gdb` trap in
  `debugger.rs`, the CPU queries in `sys/compute.rs`, and a handful of pointer
  moves in the config manager and the database pool. There are no `unsafe
  impl`s — `runtime/registry.rs` carries a comment explaining why `Dep` is
  `Sync` by inference and what the answer is if the compiler ever cannot prove
  it (`#![recursion_limit]`, not unsafety). New `unsafe` is held to that
  standard: a comment saying why, and why the safe route does not work.
