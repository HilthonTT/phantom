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
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
cargo test --workspace --all-features
cd cli && go vet ./... && go test -race ./...
```

`just check-rust` and `just check-go` run one half each.

CI runs one check `just` does not: `golangci-lint run` over `cli/`. It is not
in the recipe because it is a tool you install separately — see
[cli.md](cli.md#working-on-it). If you touched the Go tree, run it before you
push.

**Clippy runs with `-D warnings`.** A warning fails the build. The workspace is
warning-free today; keep it that way rather than leaving one for later.

## What CI does

| Workflow | Trigger | Runs |
| :--- | :--- | :--- |
| `rust.yml` | pushes to `main`, PRs touching `crates/`, `Cargo.*`, `rust-toolchain.toml`, `rustfmt.toml` | `cargo fmt --check`, `clippy -D warnings`, `cargo test --workspace`, then clippy and the tests again with `--all-features` |
| `go.yml` | pushes to `main`, PRs touching `cli/` or `.golangci.yml` | `go vet`, `golangci-lint`, `go test -race` |
| `audit.yml` | weekly, on demand, and on anything that moves the dependency set — a lockfile, a manifest, `deny.toml`, `cli/go.mod` | `cargo-deny`, `govulncheck` |
| `complement.yml` | pushes to `main` touching the server, weekly, on demand | Matrix's Complement suite against an image of the server — non-blocking, see [Complement](#complement) |
| `release.yml` | tags matching `v*` | placeholder — the CLI release and the server image are both still TODO |

Both check jobs carry a timeout — 45 minutes for Rust, 20 for Go — because a
deadlocked test would otherwise sit there until the job's own six-hour limit.

CI passes `--locked` to every cargo command that resolves. The lockfile is
committed, so a manifest change arriving without its lockfile update should
fail the job rather than be resolved away, which would leave CI certifying a
dependency graph nobody has locally. `just check` deliberately leaves the flag
off: locally you do want the lockfile to update.

`cargo-deny` enforces the licence allowlist in `deny.toml`: Apache-2.0, MIT,
BSD-2-Clause, BSD-3-Clause, ISC, Unicode-3.0, 0BSD, BlueOak-1.0.0,
CDLA-Permissive-2.0, MPL-2.0 and Zlib. A dependency under anything else fails
the audit. The entries below the first six name the crates that need them; a
dependency bump that drops the last user of one is a chance to take it out
again.

The audit is not weekly-only for that reason: a licence or advisory that a
dependency bump brings in has to fail on the pull request that bumps it, not on
the following Monday, once it is already on `main`.

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
- `phantom-service/src/net/resolver/tests.rs` — the server-name resolution steps.
- `phantom-core/tests/alloc_je.rs` — only meaningful with a jemalloc feature on.
- `phantom-server/tests/smoke.rs` and `federation.rs` — end to end, described
  below.

- `cli/internal/tui/{app,panel}/*_test.go` — rendering and truncation.

### End-to-end tests

`crates/phantom-server/tests/` spawns the real `phantom-server` binary — the
one cargo builds for the test — on a fresh database in a temporary directory
and on free loopback ports, and drives it over HTTP:

- `smoke.rs` registers the first account (which becomes admin) through the
  registration-token flow, creates a room, sends a message and sees it come
  back through `/sync`, runs `!admin users list` in `#admins` and waits for the
  server user's answer, calls a handful of `/_phantom/admin/v1/*` routes, and
  stops the server with `SIGTERM`.
- `federation.rs` starts two servers named `127.0.0.1:<port>`, each with a
  plain client port and a TLS federation port (`[global.tls] ports`) serving
  the self-signed certificate in `tests/fixtures`, has a user on one join a
  room on the other, and passes a message each way.

Both fail on a server that exits during startup, on any `panicked at` in its
log, and on a shutdown that does not exit zero. They exist for what no unit
test sees: a route registered twice (axum panics "Overlapping method route"
only once the router is assembled), a service `Dep` nothing builds, a startup
migration that fails on a fresh database. Each takes a few seconds once the
binary is built, and they run under `cargo test --workspace` like everything
else, unix only. A failure prints the server's whole log.

```sh
cargo test -p phantom-server --test smoke
cargo test -p phantom-server --test federation
```

### Complement

[Complement](https://github.com/matrix-org/complement) is the Matrix
homeserver test suite. It runs each test against fresh containers of an image
built to its contract, which `complement/` provides:

- `Dockerfile` builds the server (release, without LTO, for build time) into a
  slim image exposing 8008 for clients and 8448 for federation.
- `entrypoint.sh` takes the server name from `$SERVER_NAME` and issues the
  federation certificate from the CA Complement mounts at `/complement/ca`.
- `complement.toml` is the config: open registration with guests, federation
  with Complement's certificates accepted, no admin room, and TLS on 8448
  only.
- `run.sh` builds the image, clones Complement into `target/complement` if it
  is not there, and runs it, writing `target/complement-results/`.
  `summary.sh` turns the results into a pass/fail table (needs `jq`).

It needs Docker and Go:

```sh
just complement                     # the whole suite
just complement -run TestLogin      # one area
complement/summary.sh               # the table for the last run
```

`complement.yml` runs it in CI. phantom fails a good share of the suite today,
so that job does not fail on test failures: read the job summary and the
`complement-results` artifact instead. The intent, once the pass rate settles,
is a list of tests known to pass and a job that fails only when one of those
regresses.

### Benchmarks

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
  `diagnostics/debugger.rs`, the CPU queries in `runtime/sys/compute.rs`, and a handful of pointer
  moves in the config manager and the database pool. There are no `unsafe
  impl`s — `runtime/registry.rs` carries a comment explaining why `Dep` is
  `Sync` by inference and what the answer is if the compiler ever cannot prove
  it (`#![recursion_limit]`, not unsafety). New `unsafe` is held to that
  standard: a comment saying why, and why the safe route does not work.
