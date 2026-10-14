# Moderation

What an operator can refuse, take down and clear out, and where each one is
enforced. Every action here is an admin API route under
`/_phantom/admin/v1`, an `!admin` command calling the same handler, and,
for the common ones, a row action or `:` command in the console.

## Blocking servers

Two lists refuse a remote server, and they mean the same thing:

- `forbidden_remote_server_names` in the config file, changed by editing the
  file and reloading;
- the runtime block list, kept in the database (`servername_blocked`) and
  changed while the server runs.

Both hold regular expressions matched against the server's host, without its
port. Note that they are unanchored: `evil.example` also matches
`notevil.example`. Blocking *by server name* (the console's Block action,
`!admin federation block <server>`, or `{"server": …}` in the API) writes the
anchored pattern `^evil\.example$`, which matches that host alone. Blocking
by pattern (`--pattern`, `{"pattern": …}`) takes the expression as written.

A blocked server:

- has its federation requests refused, by the `X-Matrix` origin, before any
  handler runs;
- is sent nothing: the outgoing request path refuses it, so transactions,
  queries and fetches to it all fail without a connection;
- is not used to join through, and a room whose ID or alias names it cannot be
  joined by a local user;
- has its events refused when it is the origin of a transaction;
- has its media neither fetched nor refetched.

Every one of these checks goes through `ops::moderation::forbids`, which
consults the config list and the runtime list together. The runtime list is
held compiled in memory and recompiled only when an admin changes it, so the
check costs a read lock and a regex-set match.

| | |
| :--- | :--- |
| `GET /federation/blocks` | both lists; each pattern says whether it came from the config or the runtime list |
| `POST /federation/blocks` | `{"server": …}` or `{"pattern": …}`, optional `"reason"` |
| `DELETE /federation/blocks` | `{"pattern": …}` removes that pattern; `{"server": …}` removes every runtime pattern matching the server, and answers with the config patterns that still block it |
| `!admin federation blocks` · `block <server> [--pattern] [--reason R]` · `unblock <server> [--pattern]` | the same |
| console | Federation section: a Blocked column, and Block / Unblock on a row; `:block` and `:unblock` |

Unblocking cannot lift a block the config file makes; the answer names the
patterns responsible.

## Quarantining media

A quarantined file is kept on disk but answered as missing — `404
M_NOT_FOUND`, worded as for any file not stored here — on every download and
thumbnail route, client and federation alike, and it is never fetched again
from its server. The quarantine is recorded by mxc URI, apart from the file,
so it can be set on media never fetched (to keep it from being), and deleting
the file does not lift it.

| | |
| :--- | :--- |
| `GET /media/quarantine` | every quarantine, and whether the file is stored |
| `PUT` / `DELETE /media/{server}/{media_id}/quarantine` | quarantine one file (optional `{"reason"}`) / lift it |
| `POST /rooms/{room}/media/quarantine` | every mxc URI the room's events name, from any server, as a task; member events are passed over, since an avatar there is the member's profile |
| `POST /users/{user}/media/quarantine` | every file a local user uploaded |
| `!admin media quarantined` · `quarantine <mxc>` · `unquarantine <mxc>` · `quarantine-user <user>` · `!admin rooms quarantine-media <room>` | the same |
| console | Media section: a Quarantined column, and Quarantine / Lift quarantine on a row; `:quarantine`, `:unquarantine` |

## Server ACLs

`m.room.server_acl` is enforced on every inbound federation endpoint scoped to
a room: `/send` (each PDU, by its origin and by its sender's server, and the
typing and receipt EDUs), make/send join, leave and knock, `/invite`,
`/backfill`, `/get_missing_events`, `/state`, `/state_ids`, `/event_auth`,
`/event` and `/timestamp_to_event`. A refused server gets `403 M_FORBIDDEN`;
a refused EDU is dropped.

One divergence from the letter of the spec, kept from conduwuit: an ACL whose
`allow` list is empty is ignored rather than denying every server, since it
would otherwise lock this server out of the room as well.

## Purging history

`POST /rooms/{room}/purge_history` with `{"before_ts_ms": …}` or
`{"before_event_id": …}` deletes the room's timeline events before that point,
as a task (`purge room history`). It keeps:

- every state event, since the room's state and the auth chains of later
  events are built from them;
- the room's latest events, which the next event sent names as its parents;
- events local users sent, unless `"include_local": true` — other servers hold
  copies of remote events, but a local one exists nowhere else.

Purged events go from the timeline, the event indexes, the search index and
the relation indexes. Other servers in the room keep theirs, and a client that
pages back far enough can have them backfilled again.

`!admin rooms purge-history <room> --days N | --before-event ID | --before-ts MS [--include-local]`
does the same; the console has `:purgehistory <room> <days>`.

## Purging old media

`POST /media/purge` with `{"origin": "remote" | "local", "older_than_days": N}`
deletes the stored files of that origin stored more than N days ago, as a task
(`purge media`). The store records when a file was stored, not when it was
last served, so age is the only measure: remote copies are fetched again when
someone asks for them, local uploads are gone for good. A local purge of zero
days is refused.

`!admin media purge-remote --days N` and `purge-local --days N`; the console
has `:cleanmedia remote|local <days>`.

## Retention policies

With `retention_policy_enforcement` on (off by default), the server enforces a
room's `m.room.retention` state once an hour: timeline events older than its
`max_lifetime` are purged as above, local ones included, state kept. A
`max_lifetime` below `retention_min_lifetime_secs` (a day by default) is
raised to it, so a room admin cannot make the server discard messages as they
arrive. `min_lifetime`, and server-wide default policies, are not implemented.
