# Installation

How to get a build of phantom on your machine, from a bare system to a running
admin console.

> **Read this first.** phantom does not run as a homeserver yet.
> `crates/phantom-server` is a stub whose `main` is a `todo!()`, so the server
> binary builds and then panics if you start it. What you can build and use
> today is the workspace itself (core, database, service, macros) and the Go
> admin console, which draws its interface from placeholder data. Follow this
> guide to develop on phantom, not to host a Matrix server.

## Contents

- [What you need](#what-you-need)
- [Installing the toolchains](#installing-the-toolchains)
- [Getting the source](#getting-the-source)
- [Building](#building)
- [What you end up with](#what-you-end-up-with)
- [Optional build features](#optional-build-features)
- [Configuring](#configuring)
- [Running](#running)
- [Verifying the install](#verifying-the-install)
- [Troubleshooting](#troubleshooting)
- [Uninstalling](#uninstalling)

## What you need

| Dependency | Version | Needed for | Required? |
| :--- | :--- | :--- | :--- |
| [Rust](https://rustup.rs) | 1.97.1 | the homeserver workspace | yes |
| A C/C++ compiler | any recent gcc or clang | building the bundled RocksDB | yes |
| `libclang` | 11+ | `bindgen`, which generates the RocksDB bindings | yes |
| [Go](https://go.dev/dl/) | 1.26 or newer | the `phantom` admin CLI | only for the CLI |
| [`just`](https://github.com/casey/just) | any | the `just` recipes used throughout the docs | optional |
| `git` | any | fetching the source | yes |

Two things are worth knowing before you start.

**The Rust version is pinned, and you do not pick it.** `rust-toolchain.toml`
names 1.97.1 along with `rustfmt` and `clippy`. If you install Rust through
`rustup`, running any `cargo` command inside the repository downloads and
selects that exact toolchain for you. Installing Rust from a distribution
package instead will usually give you a different compiler with no way to
switch, which is why `rustup` is the recommended route.

**RocksDB is compiled from source.** `phantom-database` depends on
`rocksdb 0.25`, whose `librocksdb-sys` bundles RocksDB 11.8.1 and builds it as
part of `cargo build`. That is where the C++ compiler and `libclang` go, and it
is also why the first build takes several minutes and a few gigabytes of
`target/` — subsequent builds reuse it. There is no separate RocksDB package to
install and no `cmake` requirement; the bundled build drives the compiler
directly.

### Disk and time

| | First build | Rebuilds |
| :--- | :--- | :--- |
| `cargo build` (debug) | 5–15 min, ~4 GB in `target/` | seconds |
| `cargo build --release` | 10–25 min | seconds to minutes |
| `go build` | under a minute, ~100 MB module cache | seconds |

The release profile uses `lto = "thin"` and `codegen-units = 1`, which is why
it costs noticeably more than the debug profile.

## Installing the toolchains

Pick your platform. Every command below is the *system* half of the install —
Rust and Go go on afterwards, and those steps are the same everywhere.

### Debian / Ubuntu

```sh
sudo apt update
sudo apt install -y build-essential clang libclang-dev pkg-config git curl
```

### Fedora / RHEL / CentOS Stream

```sh
sudo dnf install -y gcc gcc-c++ make clang clang-devel pkgconf-pkg-config git curl
```

### Arch Linux

```sh
sudo pacman -S --needed base-devel clang git curl
```

### Alpine

```sh
sudo apk add build-base clang clang-dev clang-static git curl
```

Alpine links against musl rather than glibc. That works, but it is not what CI
builds, so treat it as unproven ground.

### macOS

```sh
xcode-select --install     # ships clang and libclang
brew install just          # optional
```

The Command Line Tools give you both the compiler and the `libclang` that
`bindgen` needs, so there is nothing else to install for the Rust side.

### Windows

Build inside [WSL2](https://learn.microsoft.com/windows/wsl/install) and follow
the Debian/Ubuntu steps above. That is what this repository is developed
against — `.vscode/settings.json` is set up for it, and it is the configuration
where the allocator features described below actually do something.

A native MSVC build is possible, but `jemalloc` and `hardened_malloc` are C
libraries with no MSVC support: `crates/phantom-core/src/alloc` gates them on
`cfg(not(target_env = "msvc"))`, so on MSVC those features stay selectable and
resolve to nothing. You get the system allocator whether you ask for one or
not.

If you clone the repository on the Windows filesystem and build from WSL, every
file access crosses the `/mnt/d`-style boundary and builds get markedly slower.
Cloning into the Linux filesystem (`~/…`) is the faster arrangement.

### Rust

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
. "$HOME/.cargo/env"
```

You do not need to choose a version. The first `cargo` command run inside the
repository installs 1.97.1 because `rust-toolchain.toml` asks for it.

### Go

Install Go 1.26 or newer from [go.dev/dl](https://go.dev/dl/), or through your
package manager if it carries a version that new:

```sh
# macOS
brew install go

# Arch
sudo pacman -S go
```

Debian and Ubuntu tend to package an older Go than `cli/go.mod` requires; the
official tarball is the reliable route there.

### `just` (optional)

Every recipe in these docs has a plain `cargo`/`go` equivalent, so `just` is a
convenience rather than a dependency.

```sh
cargo install just          # any platform with Rust already installed
brew install just           # macOS
sudo pacman -S just         # Arch
```

## Getting the source

```sh
git clone https://github.com/HilthonTT/phantom.git
cd phantom
```

## Building

With `just`:

```sh
just build          # cargo build --release, then the Go CLI into target/phantom
```

Without it:

```sh
cargo build --release
cd cli && go build -o ../target/phantom ./cmd/phantom
```

To build only one half — useful while the two are so far apart in maturity:

```sh
cargo build                                  # debug build of the workspace
cd cli && go build ./...                     # compile the CLI without installing it
```

If you only want to know that the Rust workspace still compiles, `cargo check
--workspace` does it in a fraction of the time. Note that a `check` deliberately
does **not** regenerate `phantom-example.toml` — see
[Configuring](#configuring).

## What you end up with

| Path | What it is |
| :--- | :--- |
| `target/release/phantom-server` | the homeserver binary — currently panics on start |
| `target/phantom` | the admin console (`just build` puts it here) |
| `phantom-example.toml` | documented example config, rewritten on every real `cargo build` |

Neither binary is installed system-wide by the build. Copy them where you want
them, or run them from `target/`.

## Optional build features

None of these are on by default. All of them are Cargo features, passed with
`--features`.

### Allocators

`phantom-core` builds against the system allocator unless told otherwise. Each
alternative is a C library that is compiled as part of the build, so expect the
first build with one enabled to take longer.

| Feature | Effect |
| :--- | :--- |
| `jemalloc` | use jemalloc |
| `jemalloc_conf` | jemalloc with compile-time tuning |
| `jemalloc_prof` | jemalloc with heap profiling |
| `jemalloc_stats` | jemalloc with statistics collection |
| `hardened_malloc` | use hardened_malloc — ignored if `jemalloc` is also set |

```sh
cargo build --release --features phantom-core/jemalloc_stats
```

`jemalloc` and `hardened_malloc` each define a `#[global_allocator]`, so they
can never both be active: `hardened` is gated on `not(feature = "jemalloc")`.
This is also why `--all-features` does not give you a build with both — it
enables `jemalloc`, which excludes the other.

On MSVC targets every one of these is a no-op, as described under
[Windows](#windows).

### systemd

`phantom-service` has a `systemd` feature that makes the service manager notify
systemd when the server begins reloading its configuration and when it is ready
again, so a `Type=notify` unit does not treat the reload as finished before it
is.

```sh
cargo build --release --features phantom-service/systemd
```

### Unstable and instrumentation flags

These come from `RUSTFLAGS` rather than Cargo features, and the workspace
declares them in `Cargo.toml` so that using one does not warn:

| `cfg` | What it turns on |
| :--- | :--- |
| `tokio_unstable` | tokio's unstable runtime metrics, which `phantom-core::metrics` reads |
| `unabridged` | tracing instrumentation too noisy to carry by default |
| `phantom_bench` | the `#[bench]` suites — needs a nightly compiler |

```sh
RUSTFLAGS="--cfg tokio_unstable" cargo build --release
```

## Configuring

phantom reads TOML. `phantom-example.toml` at the repository root is a fully
documented copy of every option — roughly a hundred of them — and it is
**generated**: the `#[config_example_generator]` proc macro derives it from the
`Config` struct on every real `cargo build`. Editing it is pointless, because
the next build overwrites it. Copy it and edit the copy:

```sh
cp phantom-example.toml phantom.toml
```

At minimum, set the two options the file marks as needing your attention:

```toml
[global]
server_name = "phantom.chat"
database_path = "/var/lib/phantom"
```

`server_name` is the suffix on every user and room ID this server issues, and
it cannot be changed later without wiping the database.

Configuration is layered, later sources winning:

1. the file named by `$PHANTOM_CONFIG`
2. any config paths passed on the command line
3. environment variables prefixed `PHANTOM_`, where `__` separates nested keys

So `PHANTOM_SERVER_NAME=phantom.chat` sets `server_name` without touching a
file. See [configuration.md](configuration.md) for the full picture.

## Running

The admin console runs today:

```sh
./target/phantom
```

It opens on the Overview section; press `?` for the key map and `q` to quit.
Everything it shows is placeholder data — nothing in the CLI opens a socket or
reads a config yet. [cli.md](cli.md) describes what is on screen.

The server does not:

```sh
./target/release/phantom-server
# thread 'main' panicked at crates/phantom-server/src/main.rs
# not yet implemented: wire up the conduwuit-derived server
```

That panic is the current state of the project, not a broken install.

## Verifying the install

Run the same checks CI does. If these pass, your toolchain is set up correctly:

```sh
just check
```

or, spelled out:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cd cli && go vet ./... && go test -race ./...
```

Clippy runs with `-D warnings`, so any warning fails. The workspace is
warning-free today and is meant to stay that way.

## Troubleshooting

**`fatal error: 'stdio.h' file not found`, or `linker 'cc' not found`**
No C toolchain. Install the platform packages from
[Installing the toolchains](#installing-the-toolchains).

**`Unable to find libclang`, or a `bindgen` panic during `librocksdb-sys`**
`libclang` is missing or not where `bindgen` looks. Install `libclang-dev`
(Debian/Ubuntu), `clang-devel` (Fedora), or `clang-dev` (Alpine). If it is
installed somewhere unusual, point `bindgen` at it:

```sh
export LIBCLANG_PATH=/usr/lib/llvm-18/lib
```

**`error: package requires rustc 1.97.1`, or clippy lints you have never seen**
You are building with a compiler other than the pinned one — usually a
distribution Rust taking precedence over `rustup`. Check with `rustc --version`
inside the repository; it should print 1.97.1. If it does not, install Rust via
`rustup` and make sure `~/.cargo/bin` comes first on your `PATH`.

**`go: go.mod requires go >= 1.26`**
Your Go is too old. Install 1.26 or newer from go.dev rather than from a
distribution package.

**The build is killed, or the machine swaps itself to death**
`codegen-units = 1` plus a bundled RocksDB is memory-hungry. Build with fewer
jobs (`cargo build -j 2`), or use the debug profile while developing.

**Builds are slow on WSL**
The tree is probably on the Windows filesystem. Clone into the Linux filesystem
instead. `.vscode/settings.json` also sets `rust-analyzer.cargo.targetDir` for
this reason, so the editor's checks do not contend for the same cargo lock as
your terminal build.

**`phantom-example.toml` did not change after I edited the config struct**
The generator only runs when rustc is actually linking, so `cargo check` and
rust-analyzer deliberately skip it. Run a real `cargo build`.

**rust-analyzer reports `not included in any crates` in `alloc/hardened.rs`**
Expected. `alloc::je` and `alloc::hardened` are mutually exclusive `cfg`s, and
rust-analyzer resolves one set at a time. To work on the hardened path, change
`rust-analyzer.cargo.features` in `.vscode/settings.json` to
`["hardened_malloc"]` and restart the server.

## Uninstalling

phantom installs nothing outside the repository, so removing it is removing the
clone:

```sh
cargo clean          # or just: rm -rf target
cd .. && rm -rf phantom
```

The Rust toolchain it pulled in lives under `~/.rustup` and `~/.cargo`;
`rustup self uninstall` removes both if you want them gone.
