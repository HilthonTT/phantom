# Architecture

How the workspace is put together, and why it is split the way it is.

## The shape of it

phantom is two programs in one repository: a Matrix homeserver written in Rust,
and an admin console written in Go. They do not share code, and today they do
not talk to each other either — the console draws placeholder data while the
server's HTTP surface is still unwritten.

```
                        ┌──────────────────────┐
                        │   phantom-server     │  the binary — a stub today
                        └──────────┬───────────┘
                                   │
                        ┌──────────▼───────────┐
                        │   phantom-service    │  long-lived singletons:
                        │                      │  client, config, resolver,
                        │                      │  server_state, transaction_id
                        └──────────┬───────────┘
                                   │
              ┌────────────────────┴───────────────────┐
              │                                        │
   ┌──────────▼───────────┐                 ┌──────────▼───────────┐
   │  phantom-database    │────────────────▶│    phantom-core      │
   │  RocksDB, columns,   │                 │  config, error, log, │
   │  codecs, read pool   │                 │  matrix types, sys   │
   └──────────────────────┘                 └──────────┬───────────┘
                                                       │
                                            ┌──────────▼───────────┐
                                            │   phantom-macros     │
                                            │  proc macros only    │
                                            └──────────────────────┘

   ┌──────────────────────────────────────────────────────────────┐
   │  cli/  —  Go module, the `phantom` admin console (separate)   │
   └──────────────────────────────────────────────────────────────┘
```

Dependencies point downward and never back up. A crate lower in the stack
cannot reach one above it, which is what keeps the storage engine an
implementation detail of `phantom-database` and the service graph an
implementation detail of `phantom-service`.

## The crates

### `phantom-macros`

Procedural macros, in their own crate because the language requires it —
`proc-macro = true` crates cannot export anything else.

| Macro | What it does |
| :--- | :--- |
| `#[config_example_generator]` | derives `phantom-example.toml` from the `Config` struct, and a `Display` impl that prints a running server's settings |
| `#[implement(Type)]` | wraps a free function in an `impl` block for the named receiver, so a type's methods can be spread across files |
| `#[recursion_depth]` | instrumentation for types whose `Debug` can recurse |

The example generator writes a file as a side effect, which would be
intolerable if it happened on every keystroke. `rustc.rs` reads rustc's own
command line to tell a real build from a metadata-only pass, and only writes
during the former — so `cargo check` and rust-analyzer leave the file alone.

### `phantom-core`

Everything the rest of the workspace shares. The module layout is deliberately
flat: a module is named for its subject, and a module full of extension traits
is named for the type it extends. There is no `utils` — a helper that fits
nowhere is treated as a sign the subject it belongs to has not been named yet.

| Area | Modules |
| :--- | :--- |
| The server itself | `server` (the runtime handle), `config` (what an operator set it up with), `alloc`, `metrics`, `sys` |
| Diagnostics | `error`, `result`, `log`, `debugger`, `info` |
| The protocol | `matrix` — events, PDUs, and state resolution |
| Language-level support | `arrayvec`, `bool`, `bytes`, `future`, `hash`, `json`, `macros`, `math`, `rand`, `set`, `stream`, `sync`, `text`, `time` |

Two things in here are load-bearing for everything above them.

**`Server`** is the handle a running instance is driven through: its name, the
config manager, the tokio runtime handle, the shutdown and reload flags, the
broadcast channel those signals travel on, and the logging and metrics state.
Services are built against an `Arc<Server>` and read the current config through
it rather than holding a copy.

**`Error`** is the crate's error type, and it is also why `axum` appears in
`phantom-core`'s dependencies at all: `impl IntoResponse for Error` has to live
in the crate that defines `Error`, and the orphan rule leaves it no other home.

`matrix::state_res` is phantom's own implementation of state resolution rather
than ruma's, which is why the `ruma` dependency deliberately omits the
`state-res` feature — it would only be dead weight.

### `phantom-database`

The on-disk state, and the only crate that knows the storage engine is RocksDB.
It is built in layers:

- **`Engine`** — the open database and the operations acting on it as a whole:
  flush, compaction, backup, repair, and the properties an operator queries.
- **`Map`** — one column, and where nearly all work above this crate happens.
  The typed surface is split across submodules by what it does (`get`,
  `insert`, `iter`, `keys`, `stream`, `count`, `contains`, `clear`, `compact`)
  and lands on `Map` as one flat set of methods.
- **`Database`** — the engine plus every column open on it, and what a server
  hands around. `schema.rs` names all 88 columns; a test asserts the list stays
  alphabetical and free of duplicates.
- **The codecs** — `serialize`/`deserialize` turn Rust values into keys and
  values. Iteration is in byte order, so how a key is written decides which
  ranges of it can be asked for; `Interfix` and `SEP` are the tools for
  composing those keys. Values that are a struct rather than a key-shaped tuple
  are written as CBOR (`Cbor`) or JSON (`Json`).
- **The pool** — a read that misses the block cache blocks its thread until
  storage answers. Doing that on a tokio worker would stall every other task
  sharing it, so the map layer tries the cache first and submits a miss to a
  pool of OS threads whose whole job is that wait. The pool is sized after the
  storage device rather than the CPU, because what is being waited on is the
  device's queue depth.

### `phantom-service`

A service is a long-lived singleton owning one area of the server's behaviour.
`runtime/` is the machinery they all plug into; every other module is one
service.

Currently built:

| Service | What it owns |
| :--- | :--- |
| `resolver` | turning a Matrix server name into an address, and caching the answer |
| `client` | the HTTP clients every outbound request is made through |
| `config` | re-reading the config file on `SIGUSR1` and swapping it in |
| `server_state` | the server's own identity, its secrets, and the event counter |
| `transaction_id` | what a transaction id was answered with, so a retry gets the original response |
| `rooms` | a placeholder tree — only `rooms::outlier` exists, and it is unimplemented |
| `admin` | the admin room, who counts as an admin, and the command queue |
| `pusher` | the push gateways a client registered, and the notifications sent through them |

The interesting parts of the runtime are the three problems it solves.

**Services must be able to depend on each other in both directions.** A pair of
`Arc`s pointing both ways is a cycle that never drops, so services reach each
other by name through a registry holding only weak references. The strong
references belong to `Services`; `Dep<T>` is the handle a service holds, and it
resolves lazily on first use.

**A service that panics must not take the server down.** `Manager` runs each
service's worker as a task in a `JoinSet`. A worker that returns an error or
panics is restarted after a 2.5-second backoff, and shutdown is what waits on a
worker that will not stop.

**Construction and startup are separate steps.** `Services::build` constructs
everything without starting anything, so a service built early can depend on
one built after it. Only `Services::start` spawns the workers. Build order
matters solely where one service reaches another *during construction* — the
resolver is built before the client for exactly that reason, since every client
is built against it.

The resolver is worth reading on its own. A Matrix server name is not a
hostname: resolving one follows [the spec's procedure][spec] — an IP literal is
used as-is, a name carrying a port is used as-is, and otherwise
`.well-known/matrix/server` is asked first, SRV records second, and only then
is the name itself resolved. The answer is cached in the database rather than
only in memory, because those lookups are several round trips before the first
byte of a federation request can be sent, and the result is good for hours.
`dns.rs` is what lets reqwest read that cache when it opens the connection, so
the address a server name resolved to is the address connected to.

The client service keeps one HTTP client per *kind* of request rather than one
for all of them. Each has its own connection pool and timeouts, so a push
gateway that has stopped answering cannot hold connections a federation request
needs, and a URL preview cannot wait as long as a room join legitimately does.

[spec]: https://spec.matrix.org/latest/server-server-api/#resolving-server-names

### `phantom-server`

The binary. Today it is four lines and a `todo!()`; wiring the service graph to
an axum router is the next substantial piece of work.

## The admin console

`cli/` is a separate Go module (`github.com/HilthonTT/phantom/cli`) built on
Bubble Tea v2. It is modelled on [superfile](https://github.com/yorukot/superfile),
whose panel-and-footer arrangement suits an admin console for the same reason
it suits a file manager: several listings worth reading against each other,
with the state of the session always in view underneath them.

```
┌───────────┬────────────────────────────┬──────────────┐
│  sidebar  │        workspace           │   detail     │
│           │  (one or more panels)      │  (inspector) │
├───────────┴────────────────────────────┴──────────────┤
│  taskbar  ·  summary  ·  connection                   │
└───────────────────────────────────────────────────────┘
```

Package by package: `app` is the Bubble Tea model and the layout; `sidebar`,
`workspace`, `panel`, `detail`, `inspector`, `taskbar`, `summary` and
`connection` are the regions; `modal` holds the overlays (help, confirm,
prompt); `keymap` is every binding and its help text, built from one set of
values so a key cannot be rebound without the help following; `theme` is the
styling; `resource` is the shapes the interface draws.

`sample` is the placeholder content. Nothing in the CLI reads a config, opens a
socket or touches a database — the interface is complete and the data behind it
is not. When the admin API is written, `sample` is the one package to delete.

See [cli.md](cli.md) for what the console does from a user's side.

## Conventions

**No catch-all modules.** Named after the subject, not after being leftovers.
This holds across both languages.

**Divergences from upstream are commented where they occur.** phantom is a port
of conduwuit, and where it departs from it there is a comment at the site
saying so. See [upstream-sync.md](upstream-sync.md).

**Doc comments carry the reasoning.** The module docs in this workspace explain
why a thing is shaped the way it is, not just what it does — a good deal of
this file is drawn from them. When you change a decision, change the comment
that explains it.
