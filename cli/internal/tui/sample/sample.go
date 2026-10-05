package sample

import (
	"fmt"
	"strings"

	"github.com/HilthonTT/phantom/cli/internal/tui/resource"
)

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

func Listing(s resource.Section) resource.Listing {
	switch s {
	case resource.Overview:
		return overview()
	case resource.Services:
		return services()
	case resource.API:
		return api()
	case resource.Rooms:
		return rooms()
	case resource.Users:
		return users()
	case resource.Devices:
		return devices()
	case resource.Tokens:
		return tokens()
	case resource.Federation:
		return federation()
	case resource.Appservices:
		return appservices()
	case resource.Media:
		return media()
	case resource.Tasks:
		return tasks()
	case resource.Reports:
		return reports()
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
	routedN, unwiredN, unportedN := routeCounts()

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
			row("API routes", fmt.Sprintf("%d routed, %d unwired, %d modules not ported",
				routedN, unwiredN, unportedN), resource.Held),
			row("HTTP listener", "not connected", resource.Held),
			row("Local users", "1,284", resource.NoState),
			row("Devices", "3,912, 71% cross-signed", resource.NoState),
			row("Key backups", "804 users", resource.NoState),
			row("Rooms", "312", resource.NoState),
			row("Events today", "48,910", resource.NoState),
			row("Database size", "1.8 GiB", resource.NoState),
			row("Media store", "24.6 GiB", resource.NoState),
			row("Federation", "42 servers reachable", resource.Done),
			row("Registration", "token, then email, then terms", resource.Held),
			row("Login", "password, token, appservice, OIDC", resource.Done),
			row("Appservices", fmt.Sprintf("%d registered", len(appservices().Rows)), resource.NoState),
			row("Open reports", fmt.Sprint(openReports()), resource.Held),
			row("Backup", "12 hours ago", resource.Held),
			row("Read-only mode", "off", resource.NoState),
		},
	}
}

type service struct {
	name   string
	worker bool

	unwired bool

	planned bool

	purpose string
}

var registry = []service{
	{name: "net::resolver", purpose: "a server name turned into an address"},
	{name: "net::client", purpose: "the outbound HTTP clients"},
	{name: "ops::config", worker: true, purpose: "re-reading the config on SIGUSR1"},
	{name: "ops::storage", worker: true, purpose: "the configured object-storage providers"},
	{name: "media", worker: true, purpose: "uploads, thumbnails, remote fetches"},
	{name: "rooms::membership", worker: true, purpose: "join, leave, invite, kick, ban, knock"},
	{name: "ops::moderation", purpose: "which servers this one refuses"},
	{name: "auth::rendezvous", purpose: "QR-code login (MSC4108)"},
	{name: "net::federation", purpose: "one signed request to another server"},

	{name: "rooms::alias", purpose: "the #name:server a room is reached by"},
	{name: "rooms::delete", purpose: "shutting a room down, purging"},
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

	{name: "net::server_keys", purpose: "signing keys, this server's and others'"},
	{name: "ops::server_state", purpose: "identity, secrets, the event counter"},
	{name: "accounts::sync", purpose: "parking a /sync until something happens"},
	{name: "ops::tasks", worker: true, purpose: "long admin operations, polled"},
	{name: "accounts::transaction_id", purpose: "a retry answered with the first response"},
	{name: "auth::uiaa", purpose: "interactive-auth sessions in progress"},
	{name: "accounts::account_data", purpose: "account data, global and per-room"},
	{name: "accounts::key_backups", purpose: "server-side backups of room keys"},
	{name: "ops::appservice", worker: true, purpose: "the registered appservices"},
	{name: "accounts::users", purpose: "accounts, devices, keys"},
	{name: "accounts::deactivate", purpose: "tearing an account down"},
	{name: "ops::emergency", worker: true, purpose: "the way back in when admins are locked out"},
	{name: "accounts::presence", worker: true, purpose: "who is online, and for how long"},
	{name: "accounts::profile", purpose: "display name, avatar, and the custom fields"},
	{name: "accounts::pusher", purpose: "push gateways, and what is sent through them"},
	{name: "net::sending", worker: true, purpose: "the outbound federation and push queue"},
	{name: "ops::admin", worker: true, purpose: "the admin room and its commands"},
	{name: "ops::updates", worker: true, purpose: "the announcement feed"},
	{name: "net::sendmail", purpose: "outbound SMTP, when one is configured"},
	{name: "auth::oauth", purpose: "OIDC login and OAuth2 (MSC3861)"},

	{name: "net::fetcher", worker: true, purpose: "coalesced federation fetches"},
	{name: "auth::registration_tokens", purpose: "token-gated registration"},
	{name: "auth::threepid", purpose: "email and phone bindings"},

	{name: "ops::migrations", purpose: "schema and data migrations, run at startup"},
}

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

// area is the domain folder a service lives in under phantom-service/src, which
// is the part of its registry name before "::".
func (s service) area() string {
	if area, _, ok := strings.Cut(s.name, "::"); ok {
		return area
	}
	return "core"
}

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

func serviceCounts() (built, workers, planned int) {
	for _, svc := range registry {
		switch {
		case svc.planned:
			planned++
		case svc.unwired:

		default:
			built++
			if svc.worker {
				workers++
			}
		}
	}
	return built, workers, planned
}

type room struct {
	alias, id    string
	members, ver string
	visibility   string

	kind      string
	joinRule  string
	history   string
	published bool
	encrypted bool
	threads   string
	marked    bool
}

func (r room) row() resource.Row {
	encryption, emphasis := "no", resource.Held
	if r.encrypted {
		encryption, emphasis = "yes", resource.Done
	}

	published := "no"
	if r.published {
		published = "yes"
	}

	return resource.Row{
		Cells:  []string{r.alias, r.members, r.ver, r.visibility},
		Marked: r.marked,
		Detail: []resource.Field{
			{Label: "Alias", Value: r.alias + ":phantom.chat"},
			{Label: "Room ID", Value: r.id},
			{Label: "Type", Value: r.kind},
			{Label: "Members", Value: r.members},
			{Label: "Version", Value: r.ver},
			{Label: "Visibility", Value: r.visibility},
			{Label: "Join rule", Value: r.joinRule},
			{Label: "In directory", Value: published},
			{Label: "History", Value: r.history},
			{Label: "Encrypted", Value: encryption, Emphasis: emphasis},
			{Label: "Threads", Value: r.threads},
			{Label: "Created", Value: "2026-01-04 09:12"},
			{Label: "Creator", Value: "@admin:phantom.chat"},
			{Label: "State events", Value: "1,904"},
			{Label: "Federated", Value: "yes"},
		},
	}
}

func rooms() resource.Listing {
	all := []room{
		{alias: "#general", id: "!QsWaEdRfTgYh:phantom.chat", members: "1,204", ver: "11", visibility: "public",
			kind: "room", joinRule: "public", history: "shared", published: true, encrypted: true, threads: "38"},
		{alias: "#announcements", id: "!ZxCvBnMaSdF:phantom.chat", members: "1,198", ver: "11", visibility: "public",
			kind: "room", joinRule: "public", history: "world_readable", published: true, encrypted: true, threads: "0"},
		{alias: "#phantom", id: "!SpAcEhOmEx:phantom.chat", members: "1,102", ver: "11", visibility: "public",
			kind: "space, 9 children", joinRule: "public", history: "world_readable", published: true, threads: "—"},
		{alias: "#random", id: "!PoIuYtReWq:phantom.chat", members: "874", ver: "11", visibility: "public",
			kind: "room", joinRule: "public", history: "shared", published: true, threads: "12", marked: true},
		{alias: "#support", id: "!RfVtGbYhNj:phantom.chat", members: "623", ver: "11", visibility: "public",
			kind: "room", joinRule: "knock", history: "joined", published: true, encrypted: true, threads: "51"},
		{alias: "#matrix-spec", id: "!LkJhGfDsAp:phantom.chat", members: "512", ver: "10", visibility: "public",
			kind: "room", joinRule: "public", history: "shared", published: true, threads: "7"},
		{alias: "#offtopic", id: "!TgBnHyMjUk:phantom.chat", members: "341", ver: "11", visibility: "public",
			kind: "room", joinRule: "public", history: "shared", threads: "3"},
		{alias: "#dev", id: "!MnBvCxZaSd:phantom.chat", members: "218", ver: "11", visibility: "private",
			kind: "room", joinRule: "restricted, #phantom", history: "shared", encrypted: true, threads: "22"},
		{alias: "#ops", id: "!QwErTyUiOp:phantom.chat", members: "96", ver: "11", visibility: "private",
			kind: "room", joinRule: "invite", history: "invited", encrypted: true, threads: "4", marked: true},
		{alias: "#bridge-irc", id: "!ZaQxSwCdEv:phantom.chat", members: "88", ver: "9", visibility: "public",
			kind: "room, bridged", joinRule: "public", history: "shared", published: true, threads: "0"},
		{alias: "#admins", id: "!AsDfGhJkLz:phantom.chat", members: "12", ver: "11", visibility: "private",
			kind: "room", joinRule: "invite", history: "joined", encrypted: true, threads: "1"},
	}

	rows := make([]resource.Row, 0, len(all))
	for _, r := range all {
		rows = append(rows, r.row())
	}

	return resource.Listing{
		Sort: "members, descending",
		Columns: []resource.Column{
			{Title: "Alias", Flex: true},
			{Title: "Members", Width: 9, Right: true},
			{Title: "Ver", Width: 5, Right: true},
			{Title: "Visibility", Width: 12},
		},
		Rows: rows,
	}
}

type user struct {
	id, name     string
	admin        bool
	state        string
	seen         string
	presence     string
	devices      string
	crossSigning bool
	backup       string
	email        string
	emphasis     resource.State
}

func (u user) row() resource.Row {
	admin := "no"
	if u.admin {
		admin = "yes"
	}

	signing, signed := "not set up", resource.Held
	if u.crossSigning {
		signing, signed = "set up", resource.Done
	}

	return resource.Row{
		Cells: []string{u.id, admin, u.state, u.seen},
		State: u.emphasis,
		Detail: []resource.Field{
			{Label: "User ID", Value: u.id + ":phantom.chat"},
			{Label: "Display name", Value: u.name},
			{Label: "Admin", Value: admin},
			{Label: "State", Value: u.state, Emphasis: u.emphasis},
			{Label: "Presence", Value: u.presence},
			{Label: "Last seen", Value: u.seen},
			{Label: "Email", Value: u.email},
			{Label: "Devices", Value: u.devices},
			{Label: "Cross-signing", Value: signing, Emphasis: signed},
			{Label: "Key backup", Value: u.backup},
			{Label: "Rooms joined", Value: "27"},
			{Label: "Registered", Value: "2025-11-02"},
			{Label: "Upload usage", Value: "412 MiB"},
		},
	}
}

func users() resource.Listing {
	all := []user{
		{id: "@ada", name: "Ada L.", admin: true, state: "active", seen: "2 min ago", presence: "online",
			devices: "3", crossSigning: true, backup: "version 4, 12,804 keys", email: "ada@phantom.chat", emphasis: resource.Done},
		{id: "@grace", name: "Grace H.", admin: true, state: "active", seen: "18 min ago", presence: "online · reviewing PRs",
			devices: "1", crossSigning: true, backup: "version 2, 6,410 keys", email: "grace@phantom.chat", emphasis: resource.Done},
		{id: "@alan", name: "Alan T.", state: "active", seen: "1 hour ago", presence: "unavailable",
			devices: "1", backup: "none", email: "—", emphasis: resource.Done},
		{id: "@edsger", name: "Edsger D.", state: "active", seen: "3 hours ago", presence: "offline",
			devices: "1", crossSigning: true, backup: "version 1, 980 keys", email: "edsger@example.org", emphasis: resource.Done},
		{id: "@barbara", name: "Barbara L.", state: "suspended", seen: "2 days ago", presence: "offline",
			devices: "2", crossSigning: true, backup: "version 1, 2,113 keys", email: "barbara@phantom.chat", emphasis: resource.Held},
		{id: "@donald", name: "Donald K.", state: "deactivated", seen: "41 days ago", presence: "offline",
			devices: "0", backup: "deleted", email: "—", emphasis: resource.Failed},
		{id: "@ken", name: "Ken T.", state: "active", seen: "5 hours ago", presence: "offline",
			devices: "1", backup: "none", email: "ken@phantom.chat", emphasis: resource.Done},
		{id: "@dennis", name: "Dennis R.", state: "active", seen: "6 hours ago", presence: "offline",
			devices: "2", crossSigning: true, backup: "version 3, 4,002 keys", email: "dennis@phantom.chat", emphasis: resource.Done},
		{id: "@bjarne", name: "Bjarne S.", state: "shadowbanned", seen: "9 days ago", presence: "offline",
			devices: "1", backup: "none", email: "—", emphasis: resource.Failed},
		{id: "@linus", name: "Linus T.", state: "active", seen: "12 hours ago", presence: "offline",
			devices: "4", crossSigning: true, backup: "version 1, 310 keys", email: "linus@phantom.chat", emphasis: resource.Done},
	}

	rows := make([]resource.Row, 0, len(all))
	for _, u := range all {
		rows = append(rows, u.row())
	}

	return resource.Listing{
		Sort: "last seen, newest first",
		Columns: []resource.Column{
			{Title: "User", Flex: true},
			{Title: "Admin", Width: 7},
			{Title: "State", Width: 12},
			{Title: "Last seen", Width: 14, Right: true},
		},
		Rows: rows,
	}
}

func tokens() resource.Listing {
	token := func(tok, uses, maxUses, expires string, state resource.State) resource.Row {
		limit := "none"
		if maxUses != "" {
			limit = maxUses + " uses"
		}

		return resource.Row{
			Cells: []string{tok, uses, limit, expires},
			State: state,
			Detail: []resource.Field{
				{Label: "Token", Value: tok},
				{Label: "Uses", Value: uses},
				{Label: "Max uses", Value: limit},
				{Label: "Expires", Value: expires, Emphasis: state},
				{Label: "Source", Value: "database"},
			},
		}
	}

	return resource.Listing{
		Sort: "expiry, soonest first",
		Columns: []resource.Column{
			{Title: "Token", Flex: true},
			{Title: "Uses", Width: 6, Right: true},
			{Title: "Max uses", Width: 10, Right: true},
			{Title: "Expires", Width: 20, Right: true},
		},
		Rows: []resource.Row{
			token("kR7fQ2xLmW9sTb4N", "9", "10", "in 3 hours", resource.Held),
			token("Hp3VzY8cJq1dGe6U", "2", "", "2026-09-26 18:00", resource.Done),
			token("aN5tBw0XoK4rMi7C", "14", "50", "2026-10-01 00:00", resource.Done),
			token("Zs2uEj9PfL6yDh3Q", "0", "1", "never", resource.Done),
			token("qT8gRc1VnA5kWx0S", "37", "", "never", resource.Done),
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
			entry("14:11:02", "DEBUG", "phantom_database", "opened 105 columns", resource.NoState),
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
			option("client.login_with_password", "true", "default"),
			option("client.login_via_token", "true", "default"),
			option("client.login_via_existing_session", "true", "default"),
			option("client.allow_guest_registration", "false", "default"),
			option("client.enable_set_displayname", "true", "default"),
			option("client.default_room_version", "12", "default"),
			option("client.allow_encryption", "true", "default"),
			option("client.allow_local_presence", "true", "default"),
			option("client.one_time_key_limit", "256", "default"),
			option("client.federation_keys_timeout", "8", "default"),
			option("client.client_sync_timeout_default", "30000", "default"),
			option("client.lockdown_public_room_directory", "false", "default"),
			option("client.show_all_local_users_in_user_directory", "false", "default"),
			option("client.well_known_support_email", "admin@phantom.chat", "file"),
		},
	}
}
