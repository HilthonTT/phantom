# The admin console

`phantom` is a terminal interface for looking at a running homeserver. It is a
Go program under `cli/`, separate from the Rust workspace, and it is the piece
of this project you can actually run today.

> **Sign in as an admin to see the server.** The sections are drawn from the
> admin API (`/_phantom/admin/v1`) once an admin is signed in. Until then, and
> for an account that is not an admin, they show the placeholder rows in
> `cli/internal/tui/sample`, and the listing's footer says so.

## Running it

```sh
just build          # or: cd cli && go build -o ../target/phantom ./cmd/phantom
./target/phantom
```

There are no flags and no subcommands. Run with no arguments, it opens the
console.

The interface needs a terminal at least **80×24**. Below either dimension the
panels cannot hold a row between their borders, so a warning is drawn instead
of an unreadable layout. The detail panel on the right needs **110** columns
before it appears at all — under that, those columns are better spent on the
listing.

## The layout

```
┌───────────┬──────────────────────────────────┬──────────────┐
│           │  Rooms      │  Users             │              │
│  SERVER   │─────────────┴────────────────────│   inspector  │
│  Overview │  #general:phantom.chat           │              │
│  Rooms    │  #ops:phantom.chat               │   Name  …    │
│  Users    │  #random:phantom.chat            │   Members  … │
│  …        │                                  │   Version  … │
├───────────┴──────────────────────────────────┴──────────────┤
│  tasks              │  selection      │  connection         │
└─────────────────────────────────────────────────────────────┘
```

The arrangement is modelled on [superfile](https://github.com/yorukot/superfile).
Its panel-and-footer layout suits an admin console for the same reason it suits
a file manager: several listings worth reading against each other, with the
state of the session always in view underneath them.

**The sidebar** down the left lists every section under its heading — `SERVER`
(Overview, Rooms, Users, Federation, Media) and `OPERATIONS` (Tasks, Logs,
Settings) — with a box at the top for filtering the list. Its width is fixed,
since the section names are known; widening the terminal widens the listings
rather than the furniture around them.

**The workspace** in the middle holds the listings. One listing is one tab, and
up to **three** can be open side by side, so the rooms in one and the users in
another can be read together. The tabs are peers: one has the keyboard, and the
rest keep their cursors where they were left.

**The inspector** down the right shows the row the workspace cursor is on, one
labelled field per line. It has no cursor of its own — it follows the
workspace, the way superfile's preview pane follows the file panel.

**The footer** is three boxes:

| Box | Shows |
| :--- | :--- |
| tasks | one entry per long-running operation, each with a progress bar and a cursor for picking one out |
| selection | the two or three fields worth glancing at for whatever the cursor is on — deliberately overlapping the inspector, which is only drawn on a wide terminal |
| connection | which homeserver the CLI is pointed at and whether it is answering |

The connection box is the one that is not about the selection. Wherever the
cursor is, it says what is being administered — which is what stops an
operation being run against the wrong server.

## Keys

Press `?` at any time for this list; the help menu and the bindings are built
from the same values, so a rebound key takes its help text with it.

### Movement

| Key | Does |
| :--- | :--- |
| `↑` / `k` | move up |
| `↓` / `j` | move down |
| `PgUp` / `Ctrl-U` | page up |
| `PgDn` / `Ctrl-D` | page down |
| `Home` / `g` | jump to the first row |
| `End` / `G` | jump to the last row |

### Panels

| Key | Does |
| :--- | :--- |
| `Tab` | focus the next panel |
| `Shift-Tab` | focus the previous panel |
| `n` | open another panel |
| `w` | close this panel |
| `Ctrl-→` / `L` | move focus clockwise |
| `Ctrl-←` / `H` | move focus anticlockwise |

Focus walks the sidebar, the workspace and the task bar in that order.

### Rows

| Key | Does |
| :--- | :--- |
| `Space` | mark the row under the cursor |
| `a` | mark every row |
| `A` | clear all marks |
| `Enter` | open what the cursor is on |
| `r` | reload the listing |

### Overlays

| Key | Does |
| :--- | :--- |
| `/` | filter what is listed |
| `:` | open the command prompt |
| `s` | change the sort order |
| `?` | show the help |
| `Esc` | dismiss what is open, or cancel a task |

### Session

| Key | Does |
| :--- | :--- |
| `q` / `Ctrl-C` | quit |

### Logs

These answer only while the Logs section has the keyboard.

| Key | Does |
| :--- | :--- |
| `f` | follow the log as it grows, or stop |
| `p` | pause or resume following |
| `v` | step the lowest level shown: every line, INFO, WARN, ERROR |
| `t` | show only the lines of one target module (a substring; empty clears it) |
| `m` | search the lines' message, target and span (empty clears it) |
| `d` | have the server keep its own debug lines too, or stop |

### Room inspector

| Key | Does |
| :--- | :--- |
| `i` | on a Rooms row: open the inspector on that room |
| `Tab` / `Shift-Tab` | the next or previous view: state, timeline, extremities, servers |
| `Enter` / `Space` | show or hide the selected event's JSON |
| `o` | load older timeline events; in the Audit section, older records |
| `r` | fetch the room again |
| `Esc` | close the inspector |

## The command prompt

`:` opens a command line. It lists what will eventually be typeable — nothing
happens when you press Enter yet; the prompt is there so the shape of the
interface is complete, and the handlers are the work that follows.

| Command | Intent |
| :--- | :--- |
| `room <alias>` | open a room's record |
| `inspect <room>` | open the room inspector, by ID, name or alias |
| `user <id>` | open a user's record |
| `purge <room>` | delete a room and purge it |
| `purgehistory <room> <days>` | purge remote users' messages older than |
| `cleanmedia remote\|local <days>` | purge stored media older than |
| `block <server>` · `unblock <server>` | stop or resume federating with a server |
| `deactivate <user>` | deactivate an account |
| `reactivate <user>` | reactivate a deactivated account with a new password |
| `quarantine <mxc>` · `unquarantine <mxc>` | stop or resume serving a file |
| `reload` | re-read the config file |
| `backup` | start a database backup |
| `cancel <task id>` | stop a running room shutdown or deletion |

## What the admin sections show

**API** is the server's own route table, from `GET /_phantom/admin/v1/routes`:
every method and path it registered, the audience it serves (client,
federation, media, admin, oidc, well-known), and for a route declared through
ruma the endpoint it implements and whether the path is a stable one. The
table is recorded as the router is built, so it cannot fall out of step with
what is served. Before an admin signs in the section shows the old hand-kept
list, labelled as a sample.

**Logs** reads the server's in-memory log: the last thousand lines at info
and above. Following asks every two seconds for only the lines after the
newest one held (each line carries a sequence number, and
`GET /logs?since=` answers with the ones after it), and keeps the cursor on
the newest. Pausing holds the follow without losing the place; resuming
fetches what arrived meanwhile, unless more than the server's thousand lines
went by. The level, target and text filters narrow what is drawn without
asking the server again. `d` makes the server keep phantom's own debug lines
as well (`PUT /logs/capture`); other crates' debug lines stay out, and a
restart goes back to info.

**Federation** colours each server by how talking to it goes: *reachable* in
green; *flaky* (a failure since its last answer), *retrying* or *backing off*
in amber; *failing* (five failures in a row) or *unreachable* in red. The
inspector adds the events queued for it, the last error and when, how many
requests have failed in a row, and when it last sent us a transaction. All of
it but the backoff is kept in memory, so it covers the time since startup.

**The room inspector** (`i` on a Rooms row) shows what the server holds of a
room, whoever may see it: its current state, its timeline newest first with
each event's depth and the events it follows (an event following more than
one, where the graph merged, is marked), its forward extremities (more than
one means the graph has forked; one the server lists but does not hold is
marked red), and the servers in it. It is read from
`GET /_phantom/admin/v1/rooms/{room}/state`, `…/events?limit=&from=`,
`…/extremities` and `…/servers`.

**Audit** is the admin audit log, newest first, a page of 200 at a time: every
change an admin made, through the admin API or an `!admin` command, with who,
what, to what, and how it went. It is kept in the database. The recording is
done by a layer over every admin route and by the command processor, never by
the handlers, so an admin route or command added later is recorded without
anyone remembering to. Reads are not recorded; nor are passwords, since a
command keeps only its first argument.

### Confirmation

Anything that changes the server asks first. The ones that cannot be undone
and act on one thing easily mistaken for another — deleting a room, erasing an
account, purging a server's media — ask for its name to be typed: a room's
name or ID, an account's ID, a server's name. Anything else that typed text
does not match is refused and nothing happens.

## Theming

Every colour is named once, in `cli/internal/tui/theme`. A component asks for a
style by what it is drawing — `Theme.PanelTitle`, `Theme.RowSelected` — and
never names a hex value of its own, so re-theming is a change to that one
package. The palette's field names are roles rather than hues (`Canvas`,
`Surface`, `Raised`, `Sunken`) so a light theme can fill the same fields.

There is no way to select a theme at runtime yet.

## Working on it

The console is built on [Bubble Tea v2](https://charm.land), under the
`charm.land/bubbletea/v2` module path rather than the older
`github.com/charmbracelet` one.

```sh
cd cli
go vet ./...
go test -race ./...
golangci-lint run          # errcheck, govet, ineffassign, staticcheck, unused
```

The packages, by what they draw:

| Package | Responsibility |
| :--- | :--- |
| `app` | the Bubble Tea model: owns every panel, decides which has the keyboard, lays them out |
| `sidebar` | the section navigator |
| `workspace` | the listing tabs |
| `panel` | the borders, titles and truncation every box is drawn with |
| `inspector` | the detail panel |
| `detail` | the labelled lines the inspector, summary and connection boxes are all built from |
| `taskbar`, `summary`, `connection` | the three footer boxes |
| `modal` | the overlays: help, confirm, prompt |
| `keymap` | every binding and its help text |
| `theme` | palette and styles |
| `resource` | the shapes the interface draws |
| `sample` | placeholder data — **the one package to delete** when the admin API lands |
