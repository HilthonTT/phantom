# Configuration

Where phantom's settings come from, how the example file is produced, and what
to do when you want to add an option.

## The one rule

**`phantom-example.toml` is generated. Do not edit it.**

It is written to the repository root on every real `cargo build`, derived from
the `Config` struct in `crates/phantom-core/src/config/mod.rs` by the
`#[config_example_generator]` proc macro. Each field's doc comment becomes that
option's documentation, and its `#[serde(default = "…")]` becomes the value
shown. Anything you write into the file directly is lost on the next build.

To configure a server, copy it and edit the copy. To document or add an option,
edit the struct.

## Writing a config

```sh
cp phantom-example.toml phantom.toml
```

Every option lives under a single `[global]` section. The two you must set are
the ones the generated file marks in capitals:

```toml
[global]
server_name = "phantom.chat"
database_path = "/var/lib/phantom"
```

`server_name` is the suffix on every user and room ID the server issues. It
cannot be changed later without wiping the database, and the reload path
rejects a config that changes it outright.

## Where settings come from

Sources are layered, and later ones win:

1. the file named by `$PHANTOM_CONFIG`
2. any config paths passed on the command line
3. environment variables prefixed `PHANTOM_`

Environment variables are read with `__` separating nested keys and are applied
globally, so `PHANTOM_SERVER_NAME=phantom.chat` sets `server_name` with no file
involved. This is what makes container deployments practical: ship one file and
override the handful of values that differ per environment.

```sh
PHANTOM_CONFIG=/etc/phantom/phantom.toml \
PHANTOM_PORT=8008 \
  ./phantom-server
```

## Unknown and deprecated keys

A TOML key phantom does not recognise is *not* an error. It lands in a
`catchall` dictionary on `Config`, and validation logs a warning naming it at
startup — so a typo is visible rather than silently ignored, and the server
still starts. Options that older versions of phantom accepted are listed
explicitly, so a deprecated key gets a warning that says as much rather than
being reported as an unknown one.

## Validation

`config::validate` runs once the config has deserialized, and splits its work
in two.

**Rejected outright**, because the server cannot work with them:

- an empty `server_name`, or one that is not a valid Matrix server name
- an `address`/`port` pair naming no bind address at all
- a malformed `log` filter or `log_span_events` value — built and discarded
  here purely so the failure can be attributed to the option that caused it,
  rather than surfacing at logging setup where the fallback would be silence
- database options the engine cannot fall back from, such as
  `rocksdb_max_log_files = 0`, checked here where the option can be named
  instead of deep inside the engine where the reference implementation panics
  or quietly substitutes something else

**Warned about**, because they are legal but probably not what you meant: risky
URL-preview allowlists, insecure settings such as
`allow_invalid_tls_certificates`, deprecated keys, and unknown keys.

## Reloading

`config_reload_signal` (default `true`) makes the server re-read its
configuration on `SIGUSR1`. The `config` service does the work: it loads the
file again, validates the new config against the running one, and swaps it in.

Only `server_name` is fixed for the life of the process — a reload that changes
it is refused. Every other option is re-read. On platforms with no `SIGUSR1`
the option has no effect.

Build `phantom-service` with its `systemd` feature and the manager will also
notify systemd that it is reloading and, afterwards, that it is ready again, so
a `Type=notify` unit does not treat the reload as finished before it is.

## Adding or documenting an option

Everything happens in `crates/phantom-core/src/config/mod.rs`.

```rust
/// What this option does, in the words an operator should read.
///
/// A second paragraph if the first is not enough.
///
/// default: true
#[serde(default = "true_fn")]
pub allow_something: bool,
```

The doc comment *is* the documentation — it appears verbatim in
`phantom-example.toml`. Three directives are understood, each on a line of its
own:

| Directive | Effect |
| :--- | :--- |
| `default: <text>` | overrides the value shown in the example file |
| `display: hidden` | omits the option when a running server prints its config |
| `display: sensitive` | masks the value as `***********` instead of printing it |

Use `display: sensitive` for anything an operator would not want in a log:
registration tokens, TURN secrets, and the like.

After editing, run a real `cargo build`. A `cargo check` will not regenerate
the file — the generator reads rustc's own command line and only writes when
rustc is actually linking, so editors re-checking on every keystroke do not
rewrite the file continuously. Commit the regenerated
`phantom-example.toml` along with your change.

## What can be configured

There are around a hundred options. The generated file is the reference; this
is a map of the territory so you know what to look for.

| Group | Covers |
| :--- | :--- |
| Identity and listeners | `server_name`, `address`, `port` |
| Database | `database_path`, backups (`database_backup_path`, `database_backups_to_keep`), cache sizes, the read pool (`db_pool_*`) |
| RocksDB | compression, compaction, direct I/O, checksums, logging, recovery and repair, read-only and secondary modes — everything prefixed `rocksdb_` |
| Streams | `stream_width_scale`, `stream_amplification` |
| Logging and metrics | `log`, `log_colors`, `log_span_events`, `log_filter_regex`, `log_thread_ids`, `allow_metrics` |
| Registration and users | `registration_token`, `registration_token_file`, `new_user_displayname_suffix`, `forbidden_usernames`, `forbidden_alias_names` |
| Rooms and federation | `allow_room_creation`, `allow_public_room_directory_over_federation`, `allow_device_name_federation`, `trusted_servers`, `federation_loopback` |
| TURN | `turn_username`, `turn_password`, `turn_uris`, `turn_secret`, `turn_secret_file`, `turn_ttl` |
| URL previews | the `url_preview_*` allowlists and denylists, spider limits, and bound interface |
| Networking | `proxy`, `ip_range_denylist`, the per-client timeouts (`request_*`, `well_known_*`, `federation_*`, `sender_*`, `appservice_*`, `pusher_*`), and the `gzip`/`brotli`/`zstd` compression switches |
| DNS | `dns_cache_entries`, `dns_min_ttl`, `dns_min_ttl_nxdomain`, `dns_attempts`, `dns_timeout`, `dns_tcp_fallback`, `query_over_tcp_only`, `query_all_nameservers`, `ip_lookup_strategy` |
| Process | `config_reload_signal`, `allow_check_for_updates`, `allow_invalid_tls_certificates` |

The timeout groups exist because the client service keeps one HTTP client per
kind of outbound request rather than one for all of them — see
[architecture.md](architecture.md).
