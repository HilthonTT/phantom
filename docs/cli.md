# The admin console

`phantom` is a terminal interface for looking at a running homeserver. It is a
Go program under `cli/`, separate from the Rust workspace, and it is the piece
of this project you can actually run today.

> **Nothing behind it is real yet.** The console does not read a config, open a
> socket or touch a database. Every value it draws comes from
> `cli/internal/tui/sample`, which exists so the layout could be built and
> looked at before the admin API was written. Keys move the cursor and open the
> overlays; the operations they would eventually start are not implemented.

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

## The command prompt

`:` opens a command line. It lists what will eventually be typeable — nothing
happens when you press Enter yet; the prompt is there so the shape of the
interface is complete, and the handlers are the work that follows.

| Command | Intent |
| :--- | :--- |
| `room <alias>` | open a room's record |
| `user <id>` | open a user's record |
| `purge <room> <days>` | purge history older than |
| `block <server>` | stop federating with a server |
| `deactivate <user>` | deactivate an account |
| `quarantine <media>` | quarantine a media item |
| `reload` | re-read the config file |
| `backup` | start a database backup |

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
