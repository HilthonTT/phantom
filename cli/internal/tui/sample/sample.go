// Package sample is the placeholder content the interface is drawn with.
//
// None of it is real: there is no homeserver behind any of these values, and
// nothing here reads a config, opens a socket or touches a database. It exists
// so the layout can be built and looked at before the admin API is written,
// and it is the one package to delete when that happens.
package sample

import (
	"fmt"
	"strings"

	"github.com/HilthonTT/phantom/cli/internal/tui/resource"
)

// Server is the homeserver the connection box reports on.
func Server() resource.Server {
	return resource.Server{
		Name:    "phantom.chat",
		URL:     "https://phantom.chat:8448",
		Version: "phantom 0.1.0",
		Admin:   "@admin:phantom.chat",
		State:   resource.Done,
		Status:  "connected",
		Facts: []resource.Field{
			{Label: "Uptime", Value: "6d 04:11"},
			{Label: "Database", Value: "1.8 GiB"},
			{Label: "Federating", Value: "42 servers"},
		},
	}
}

// Tasks are the long-running operations the task bar draws.
func Tasks() []resource.Task {
	return []resource.Task{
		{
			Name:     "Purge history",
			State:    resource.Running,
			Progress: 0.62,
			Note:     "#general:phantom.chat · 41k of 66k events",
		},
		{
			Name:     "Rebuild search index",
			State:    resource.Running,
			Progress: 0.18,
			Note:     "shard 2 of 8",
		},
		{
			Name:     "Media cleanup",
			State:    resource.Done,
			Progress: 1,
			Note:     "freed 412 MiB",
		},
		{
			Name:     "Federation backfill",
			State:    resource.Failed,
			Progress: 0.44,
			Note:     "matrix.example.org timed out",
		},
		{
			Name:     "Nightly backup",
			State:    resource.Held,
			Progress: 0,
			Note:     "waiting for the write lock",
		},
	}
}

// Listing is the table for one section.
func Listing(s resource.Section) resource.Listing {
	switch s {
	case resource.Overview:
		return overview()
	case resource.Services:
		return services()
	case resource.Rooms:
		return rooms()
	case resource.Users:
		return users()
	case resource.Federation:
		return federation()
	case resource.Media:
		return media()
	case resource.Tasks:
		return tasks()
	case resource.Logs:
		return logs()
	case resource.Settings:
		return settings()
	default:
		return resource.Listing{}
	}
}

func overview() resource.Listing {
	built, workers, planned := serviceCounts()

	row := func(k, v string, state resource.State) resource.Row {
		return resource.Row{
			Cells: []string{k, v},
			State: state,
			Detail: []resource.Field{
				{Label: "Property", Value: k},
				{Label: "Value", Value: v, Emphasis: state},
			},
		}
	}

	return resource.Listing{
		Sort: "as reported",
		Columns: []resource.Column{
			{Title: "Property", Width: 22},
			{Title: "Value", Flex: true},
		},
		Rows: []resource.Row{
			row("Server name", "phantom.chat", resource.NoState),
			row("Version", "phantom 0.1.0", resource.NoState),
			row("Uptime", "6d 04:11", resource.NoState),
			row("Services", fmt.Sprintf("%d built, %d with workers, %d planned",
				built, workers, planned), resource.Done),
			row("Local users", "1,284", resource.NoState),
			row("Rooms", "312", resource.NoState),
			row("Events today", "48,910", resource.NoState),
			row("Database size", "1.8 GiB", resource.NoState),
			row("Media store", "24.6 GiB", resource.NoState),
			row("Federation", "42 servers reachable", resource.Done),
			row("Registration", "token required", resource.Held),
			row("Backup", "12 hours ago", resource.Held),
			row("Read-only mode", "off", resource.NoState),
		},
	}
}

// service is one row of the services listing: the name the runtime registry
// knows it by, whether the manager runs a worker for it, and a phrase for what
// it owns.
//
// There is no separate worker column, because it would say nothing the state
// does not: a service the manager runs a worker for is the one that reads as
// running, and one without a worker is built, answers calls, and is otherwise
// idle.
type service struct {
	name   string
	worker bool

	// unwired marks a module that implements the service contract but that
	// `Services::build` does not construct, so nothing can reach it.
	unwired bool

	// planned marks a service that is designed but not yet written. It has no
	// module in the tree, so the worker and unwired flags say nothing about it.
	planned bool

	purpose string
}

// registry is every service in `phantom-service`, in the order
// `runtime/services.rs` builds them — which is why the room services sit
// between `federation` and `server_keys` rather than in a block of their own.
//
// Anything not built comes after them, since it has no place in that order:
// first what is written but unreachable, then what is planned.
//
// The planned entries are listed rather than omitted so that the section shows
// the whole shape of the server. An operator reading it sees that OAuth login
// exists as an intention and is not running, instead of wondering whether the
// console simply failed to report it.
var registry = []service{
	{name: "resolver", purpose: "a server name turned into an address"},
	{name: "client", purpose: "the outbound HTTP clients"},
	{name: "config", worker: true, purpose: "re-reading the config on SIGUSR1"},
	{name: "media", worker: true, purpose: "uploads, thumbnails, remote fetches"},
	{name: "moderation", purpose: "which servers this one refuses"},
	{name: "federation", purpose: "one signed request to another server"},

	{name: "rooms::alias", purpose: "the #name:server a room is reached by"},
	{name: "rooms::directory", purpose: "the public room directory"},
	{name: "rooms::event_handler", purpose: "what an event another server sent means"},
	{name: "rooms::short", purpose: "long identifiers mapped to compact ones"},
	{name: "rooms::spaces", purpose: "rooms whose purpose is to hold rooms"},
	{name: "rooms::state", purpose: "the current state version, and extremities"},
	{name: "rooms::state_accessor", purpose: "reading state, and who may see what"},
	{name: "rooms::state_cache", purpose: "membership, denormalized both ways"},
	{name: "rooms::state_compressor", purpose: "state stored as a stack of diffs"},
	{name: "rooms::search", purpose: "full-text search over messages"},
	{name: "rooms::read_receipt", purpose: "how far each user has read"},
	{name: "rooms::timeline", purpose: "the PDUs of a room, in order"},
	{name: "rooms::auth_chain", purpose: "every event authorizing an event"},
	{name: "rooms::lazy_loading", purpose: "which members a device has been told of"},
	{name: "rooms::metadata", purpose: "known, banned or disabled"},
	{name: "rooms::outlier", purpose: "events accepted but not yet placed"},
	{name: "rooms::pdu_metadata", purpose: "relations, and what has been referenced"},
	{name: "rooms::threads", purpose: "threaded replies, and who took part"},
	{name: "rooms::typing", purpose: "who is typing, and when they stop"},
	{name: "rooms::user", purpose: "one user's unread counters in one room"},
	{name: "rooms::retention", worker: true, purpose: "originals of redacted events"},

	{name: "server_keys", purpose: "signing keys, this server's and others'"},
	{name: "server_state", purpose: "identity, secrets, the event counter"},
	{name: "sync", purpose: "parking a /sync until something happens"},
	{name: "transaction_id", purpose: "a retry answered with the first response"},
	{name: "account_data", purpose: "account data, global and per-room"},
	{name: "key_backups", purpose: "server-side backups of room keys"},
	{name: "appservice", worker: true, purpose: "the registered appservices"},
	{name: "users", purpose: "accounts, devices, keys, profiles"},
	{name: "emergency", worker: true, purpose: "the way back in when admins are locked out"},
	{name: "presence", worker: true, purpose: "who is online, and for how long"},
	{name: "pusher", purpose: "push gateways, and what is sent through them"},
	{name: "sending", worker: true, purpose: "the outbound federation and push queue"},
	{name: "admin", worker: true, purpose: "the admin room and its commands"},
	{name: "updates", worker: true, purpose: "the announcement feed"},
	{name: "sendmail", purpose: "outbound SMTP, when one is configured"},
	{name: "deactivate", purpose: "tearing an account down"},

	{name: "uiaa", unwired: true, purpose: "interactive-auth sessions in progress"},

	{name: "membership", planned: true, purpose: "join, leave, invite, kick, ban"},
	{name: "oauth", planned: true, purpose: "OIDC login and OAuth2 (MSC3861)"},
	{name: "threepid", planned: true, purpose: "email and phone bindings"},
	{name: "registration_tokens", planned: true, purpose: "token-gated registration"},
	{name: "rendezvous", planned: true, purpose: "QR-code login (MSC4108)"},
	{name: "storage", planned: true, purpose: "object storage behind media"},
	{name: "fetcher", planned: true, purpose: "coalesced federation fetches"},
	{name: "tasks", planned: true, purpose: "long admin operations, polled"},
	{name: "migrations", planned: true, purpose: "schema and data migrations"},
	{name: "rooms::delete", planned: true, purpose: "shutting a room down, purging"},
}

// state is how the listing reports one service, and the word it prints.
func (s service) state() (resource.State, string) {
	switch {
	case s.planned:
		return resource.NoState, "planned"
	case s.unwired:
		return resource.Held, "unwired"
	case s.worker:
		return resource.Running, "running"
	default:
		return resource.Done, "ready"
	}
}

// area is the half of the tree a service lives in, since the room services are
// reached as `services.rooms.x` rather than off the top level.
func (s service) area() string {
	if strings.HasPrefix(s.name, "rooms::") {
		return "rooms"
	}
	return "core"
}

// yesNo answers a question about a module in the tree. A planned service has
// no module, so the question does not apply to it and the answer is a dash
// rather than a "no" that would read as a fact about something that exists.
func (s service) yesNo(b bool) string {
	switch {
	case s.planned:
		return "—"
	case b:
		return "yes"
	default:
		return "no"
	}
}

func services() resource.Listing {
	rows := make([]resource.Row, 0, len(registry))

	for _, svc := range registry {
		state, word := svc.state()

		rows = append(rows, resource.Row{
			Cells: []string{svc.name, word, svc.purpose},
			State: state,
			Detail: []resource.Field{
				{Label: "Service", Value: svc.name},
				{Label: "State", Value: word, Emphasis: state},
				{Label: "Area", Value: svc.area()},
				{Label: "Worker", Value: svc.yesNo(svc.worker)},
				{Label: "Registered", Value: svc.yesNo(!svc.unwired)},
				{Label: "Purpose", Value: svc.purpose},
			},
		})
	}

	return resource.Listing{
		Sort: "build order",
		Columns: []resource.Column{
			{Title: "Service", Width: 24},
			{Title: "State", Width: 9},
			{Title: "Purpose", Flex: true},
		},
		Rows: rows,
	}
}

// serviceCounts is how many services are built, how many the manager runs a
// worker for, and how many are still to be written. They are counted rather
// than written down so the overview cannot drift from the listing.
func serviceCounts() (built, workers, planned int) {
	for _, svc := range registry {
		switch {
		case svc.planned:
			planned++
		case svc.unwired:
			// Written, but nothing constructs it, so it is not built either.
		default:
			built++
			if svc.worker {
				workers++
			}
		}
	}
	return built, workers, planned
}

func rooms() resource.Listing {
	room := func(alias, id, members, ver, vis string, encrypted bool, marked bool) resource.Row {
		encryption := "no"
		emphasis := resource.Held
		if encrypted {
			encryption, emphasis = "yes", resource.Done
		}

		return resource.Row{
			Cells:  []string{alias, members, ver, vis},
			Marked: marked,
			Detail: []resource.Field{
				{Label: "Alias", Value: alias + ":phantom.chat"},
				{Label: "Room ID", Value: id},
				{Label: "Members", Value: members},
				{Label: "Version", Value: ver},
				{Label: "Visibility", Value: vis},
				{Label: "Encrypted", Value: encryption, Emphasis: emphasis},
				{Label: "Created", Value: "2026-01-04 09:12"},
				{Label: "Creator", Value: "@admin:phantom.chat"},
				{Label: "State events", Value: "1,904"},
				{Label: "Federated", Value: "yes"},
			},
		}
	}

	return resource.Listing{
		Sort: "members, descending",
		Columns: []resource.Column{
			{Title: "Alias", Flex: true},
			{Title: "Members", Width: 9, Right: true},
			{Title: "Ver", Width: 5, Right: true},
			{Title: "Visibility", Width: 12},
		},
		Rows: []resource.Row{
			room("#general", "!QsWaEdRfTgYh:phantom.chat", "1,204", "11", "public", true, false),
			room("#announcements", "!ZxCvBnMaSdF:phantom.chat", "1,198", "11", "public", true, false),
			room("#random", "!PoIuYtReWq:phantom.chat", "874", "11", "public", false, true),
			room("#matrix-spec", "!LkJhGfDsAp:phantom.chat", "512", "10", "public", false, false),
			room("#dev", "!MnBvCxZaSd:phantom.chat", "218", "11", "private", true, false),
			room("#ops", "!QwErTyUiOp:phantom.chat", "96", "11", "private", true, true),
			room("#admins", "!AsDfGhJkLz:phantom.chat", "12", "11", "private", true, false),
			room("#bridge-irc", "!ZaQxSwCdEv:phantom.chat", "88", "9", "public", false, false),
			room("#offtopic", "!TgBnHyMjUk:phantom.chat", "341", "11", "public", false, false),
			room("#support", "!RfVtGbYhNj:phantom.chat", "623", "11", "public", true, false),
		},
	}
}

func users() resource.Listing {
	user := func(id, admin, state, seen string, emphasis resource.State) resource.Row {
		return resource.Row{
			Cells: []string{id, admin, state, seen},
			State: emphasis,
			Detail: []resource.Field{
				{Label: "User ID", Value: id + ":phantom.chat"},
				{Label: "Display name", Value: "Ada L."},
				{Label: "Admin", Value: admin},
				{Label: "State", Value: state, Emphasis: emphasis},
				{Label: "Last seen", Value: seen},
				{Label: "Devices", Value: "3"},
				{Label: "Rooms joined", Value: "27"},
				{Label: "Registered", Value: "2025-11-02"},
				{Label: "Upload usage", Value: "412 MiB"},
			},
		}
	}

	return resource.Listing{
		Sort: "last seen, newest first",
		Columns: []resource.Column{
			{Title: "User", Flex: true},
			{Title: "Admin", Width: 7},
			{Title: "State", Width: 12},
			{Title: "Last seen", Width: 14, Right: true},
		},
		Rows: []resource.Row{
			user("@ada", "yes", "active", "2 min ago", resource.Done),
			user("@grace", "yes", "active", "18 min ago", resource.Done),
			user("@alan", "no", "active", "1 hour ago", resource.Done),
			user("@edsger", "no", "active", "3 hours ago", resource.Done),
			user("@barbara", "no", "suspended", "2 days ago", resource.Held),
			user("@donald", "no", "deactivated", "41 days ago", resource.Failed),
			user("@ken", "no", "active", "5 hours ago", resource.Done),
			user("@dennis", "no", "active", "6 hours ago", resource.Done),
			user("@bjarne", "no", "shadowbanned", "9 days ago", resource.Failed),
			user("@linus", "no", "active", "12 hours ago", resource.Done),
		},
	}
}

func federation() resource.Listing {
	peer := func(server, status, latency, contact string, state resource.State) resource.Row {
		return resource.Row{
			Cells: []string{server, status, latency, contact},
			State: state,
			Detail: []resource.Field{
				{Label: "Server", Value: server},
				{Label: "Status", Value: status, Emphasis: state},
				{Label: "Latency", Value: latency},
				{Label: "Last contact", Value: contact},
				{Label: "Resolved via", Value: ".well-known"},
				{Label: "Address", Value: "203.0.113.17:8448"},
				{Label: "Signing key", Value: "ed25519:a_XyZq"},
				{Label: "Queued PDUs", Value: "0"},
			},
		}
	}

	return resource.Listing{
		Sort: "status, then latency",
		Columns: []resource.Column{
			{Title: "Server", Flex: true},
			{Title: "Status", Width: 12},
			{Title: "Latency", Width: 9, Right: true},
			{Title: "Last contact", Width: 15, Right: true},
		},
		Rows: []resource.Row{
			peer("matrix.org", "reachable", "84 ms", "just now", resource.Done),
			peer("mozilla.org", "reachable", "112 ms", "1 min ago", resource.Done),
			peer("kde.org", "reachable", "96 ms", "2 min ago", resource.Done),
			peer("gnome.org", "reachable", "134 ms", "4 min ago", resource.Done),
			peer("matrix.example.org", "timed out", "—", "2 hours ago", resource.Failed),
			peer("chat.example.net", "backing off", "—", "26 min ago", resource.Held),
			peer("fosdem.org", "reachable", "72 ms", "1 min ago", resource.Done),
			peer("tchncs.de", "reachable", "148 ms", "3 min ago", resource.Done),
			peer("envs.net", "blocked", "—", "never", resource.Failed),
		},
	}
}

func media() resource.Listing {
	item := func(id, size, kind, uploader string) resource.Row {
		return resource.Row{
			Cells: []string{id, size, kind, uploader},
			Detail: []resource.Field{
				{Label: "Media ID", Value: "mxc://phantom.chat/" + id},
				{Label: "Size", Value: size},
				{Label: "Type", Value: kind},
				{Label: "Uploader", Value: uploader + ":phantom.chat"},
				{Label: "Uploaded", Value: "2026-08-21 14:03"},
				{Label: "Quarantined", Value: "no"},
				{Label: "Thumbnails", Value: "3"},
				{Label: "Room", Value: "#general:phantom.chat"},
			},
		}
	}

	return resource.Listing{
		Sort: "size, largest first",
		Columns: []resource.Column{
			{Title: "Media ID", Flex: true},
			{Title: "Size", Width: 10, Right: true},
			{Title: "Type", Width: 14},
			{Title: "Uploader", Width: 12},
		},
		Rows: []resource.Row{
			item("kTqPwSxRdYfGhJ", "148.2 MiB", "video/mp4", "@ada"),
			item("mNbVcXzLkJhGfD", "96.4 MiB", "video/webm", "@grace"),
			item("qWeRtYuIoPaSdF", "24.1 MiB", "application/pdf", "@alan"),
			item("zXcVbNmAsDfGhJ", "12.8 MiB", "image/png", "@edsger"),
			item("pLoKiJuHyGtFrD", "8.2 MiB", "image/jpeg", "@ken"),
			item("aZsXdCfVgBhNjM", "4.6 MiB", "audio/ogg", "@dennis"),
			item("wSxEdCrFvTgBnH", "2.1 MiB", "image/webp", "@linus"),
			item("eDcRfVtGbYhNuJ", "812 KiB", "image/gif", "@barbara"),
		},
	}
}

func tasks() resource.Listing {
	rows := make([]resource.Row, 0, len(Tasks()))
	started := []string{"14:02", "13:47", "12:10", "11:55", "03:00"}

	for i, t := range Tasks() {
		rows = append(rows, resource.Row{
			Cells: []string{t.Name, stateWord(t.State), started[i%len(started)]},
			State: t.State,
			Detail: []resource.Field{
				{Label: "Task", Value: t.Name},
				{Label: "State", Value: stateWord(t.State), Emphasis: t.State},
				{Label: "Detail", Value: t.Note},
				{Label: "Started", Value: started[i%len(started)]},
				{Label: "Requested by", Value: "@admin:phantom.chat"},
				{Label: "Cancellable", Value: "yes"},
			},
		})
	}

	return resource.Listing{
		Sort: "running first",
		Columns: []resource.Column{
			{Title: "Task", Flex: true},
			{Title: "State", Width: 12},
			{Title: "Started", Width: 10, Right: true},
		},
		Rows: rows,
	}
}

func stateWord(s resource.State) string {
	switch s {
	case resource.Running:
		return "running"
	case resource.Done:
		return "done"
	case resource.Failed:
		return "failed"
	case resource.Held:
		return "held"
	default:
		return ""
	}
}

func logs() resource.Listing {
	entry := func(at, level, target, msg string, state resource.State) resource.Row {
		return resource.Row{
			Cells: []string{at, level, target, msg},
			State: state,
			Detail: []resource.Field{
				{Label: "Time", Value: "2026-08-27 " + at},
				{Label: "Level", Value: level, Emphasis: state},
				{Label: "Target", Value: target},
				{Label: "Message", Value: msg},
				{Label: "Thread", Value: "tokio-runtime-worker"},
				{Label: "Span", Value: "resolve{server=matrix.org}"},
			},
		}
	}

	return resource.Listing{
		Sort: "newest first",
		Columns: []resource.Column{
			{Title: "Time", Width: 10},
			{Title: "Level", Width: 7},
			{Title: "Target", Width: 20},
			{Title: "Message", Flex: true},
		},
		Rows: []resource.Row{
			entry("14:11:02", "INFO", "phantom_service", "services startup complete", resource.Done),
			entry("14:11:02", "DEBUG", "phantom_database", "opened 92 columns", resource.NoState),
			entry("14:10:58", "WARN", "phantom_service", "well-known for example.org is 14 KiB; ignoring", resource.Held),
			entry("14:10:57", "INFO", "phantom_service", "resolved matrix.org to 203.0.113.17:8448", resource.NoState),
			entry("14:10:44", "ERROR", "phantom_service", "federation send to example.org timed out", resource.Failed),
			entry("14:10:31", "INFO", "phantom_core", "config reloaded from phantom.toml", resource.NoState),
			entry("14:09:58", "DEBUG", "phantom_database", "compaction finished in 1.2s", resource.NoState),
			entry("14:09:12", "INFO", "phantom_service", "purge history started for !QsWaEd", resource.NoState),
		},
	}
}

func settings() resource.Listing {
	option := func(key, value, origin string) resource.Row {
		return resource.Row{
			Cells: []string{key, value, origin},
			Detail: []resource.Field{
				{Label: "Key", Value: key},
				{Label: "Value", Value: value},
				{Label: "Source", Value: origin},
				{Label: "Default", Value: "see phantom-example.toml"},
				{Label: "Reloadable", Value: "yes"},
			},
		}
	}

	return resource.Listing{
		Sort: "key",
		Columns: []resource.Column{
			{Title: "Key", Width: 32},
			{Title: "Value", Flex: true},
			{Title: "Source", Width: 12},
		},
		Rows: []resource.Row{
			option("server_name", "phantom.chat", "file"),
			option("address", `["127.0.0.1", "::1"]`, "file"),
			option("port", "8008", "file"),
			option("allow_registration", "false", "default"),
			option("registration_token", "(set)", "file"),
			option("max_request_size", "20971520", "default"),
			option("allow_federation", "true", "file"),
			option("trusted_servers", `["matrix.org"]`, "file"),
			option("log", "info", "env"),
			option("database_backend", "rocksdb", "default"),
			option("db_cache_capacity_mb", "512", "file"),
			option("allow_public_room_directory_over_federation", "true", "file"),
		},
	}
}
