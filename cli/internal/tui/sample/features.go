package sample

import (
	"fmt"

	"github.com/HilthonTT/phantom/cli/internal/tui/resource"
)

type endpoint struct {
	group  string
	module string
	routes int

	status routeState

	covers string
}

type routeState int

const (
	routed routeState = iota
	unwired
	unported
)

// endpoints mirrors crates/phantom-api: a group is routed when its module's
// register() mounts it, unwired when the handlers are on disk but register()
// leaves them out, and unported when tuwunel's version has not been copied
// over. Like the services registry, nothing checks it against the crate.
var endpoints = []endpoint{
	{group: "client", module: "versions", routes: 1, covers: "the spec versions and unstable features offered"},
	{group: "client", module: "well_known", routes: 2, covers: ".well-known/matrix/client and support"},
	{group: "client", module: "capabilities", routes: 1, covers: "what a client may change on this server"},
	{group: "client", module: "register", routes: 3, covers: "registration, username and token checks"},
	{group: "client", module: "session", routes: 6, covers: "login types, password/token/appservice login, refresh, logout"},
	{group: "client", module: "account", routes: 3, covers: "whoami, password change, deactivation"},
	{group: "client", module: "account::3pid", routes: 8, covers: "email bindings, validation tokens and the validate page"},
	{group: "client", module: "account_data", routes: 4, covers: "global and per-room account data"},
	{group: "client", module: "tag", routes: 3, covers: "room tags such as favourites"},
	{group: "client", module: "filter", routes: 2, covers: "stored sync filters"},
	{group: "client", module: "profile", routes: 4, covers: "display name, avatar and custom fields"},
	{group: "client", module: "presence", routes: 2, covers: "online, unavailable, offline and status messages"},
	{group: "client", module: "typing", routes: 1, covers: "typing notifications"},
	{group: "client", module: "device", routes: 5, covers: "listing, renaming and deleting devices"},
	{group: "client", module: "keys", routes: 6, covers: "device keys, one-time keys, cross-signing, signatures"},
	{group: "client", module: "backup", routes: 14, covers: "server-side room key backups"},
	{group: "client", module: "to_device", routes: 1, covers: "messages sent straight to devices"},
	{group: "client", module: "user_directory", routes: 1, covers: "searching for users"},
	{group: "client", module: "unstable", routes: 1, covers: "mutual rooms (MSC2666)"},
	{group: "client", module: "openid", routes: 1, covers: "OpenID tokens for widgets and integrations"},
	{group: "client", module: "voip", routes: 1, covers: "TURN server credentials"},
	{group: "client", module: "appservice", routes: 1, covers: "appservice ping"},
	{group: "client", module: "thirdparty", routes: 6, covers: "bridged protocols, users and locations"},
	{group: "client", module: "report", routes: 3, covers: "abuse reports on events, rooms and users"},

	{group: "client", module: "room", routes: 7, covers: "create, upgrade, summary, single events, aliases, timestamps"},
	{group: "client", module: "membership", routes: 12, covers: "join, leave, invite, kick, ban, unban, knock, forget, members"},
	{group: "client", module: "send", routes: 1, covers: "sending message events"},
	{group: "client", module: "message", routes: 1, covers: "paginating a room's messages"},
	{group: "client", module: "state", routes: 7, covers: "reading and sending state events"},
	{group: "client", module: "redact", routes: 1, covers: "redacting events"},
	{group: "client", module: "relations", routes: 3, covers: "reactions, edits and other relations"},
	{group: "client", module: "threads", routes: 1, covers: "listing a room's threads"},
	{group: "client", module: "context", routes: 1, covers: "events around an event"},
	{group: "client", module: "events", routes: 1, covers: "the legacy events stream"},
	{group: "client", module: "read_marker", routes: 2, covers: "read receipts and fully-read markers"},
	{group: "client", module: "push", routes: 12, covers: "pushers, push rules and notifications"},
	{group: "client", module: "alias", routes: 3, covers: "room aliases"},
	{group: "client", module: "directory", routes: 4, covers: "the public room directory"},
	{group: "client", module: "space", routes: 1, covers: "space hierarchies"},
	{group: "client", module: "search", routes: 1, covers: "full-text message search"},
	{group: "client", module: "sync", routes: 2, covers: "/sync v3 and sliding sync (MSC4186)"},
	{group: "client", module: "media", routes: 8, covers: "authenticated upload, download, thumbnails, previews"},
	{group: "client", module: "media_legacy", routes: 5, covers: "the deprecated unauthenticated media endpoints"},
	{group: "client", module: "phantom", routes: 2, covers: "server version and local user count"},

	{group: "federation", module: "server", routes: 31, covers: "the server-server API"},
	{group: "oidc", module: "oidc", routes: 20, covers: "the built-in OIDC provider (MSC3861)"},
}

func (e endpoint) state() (resource.State, string) {
	switch e.status {
	case unwired:
		return resource.Held, "unwired"
	case unported:
		return resource.NoState, "not ported"
	default:
		return resource.Done, "routed"
	}
}

func (e endpoint) count() string {
	if e.status == unported {
		return "—"
	}

	return fmt.Sprint(e.routes)
}

func api() resource.Listing {
	rows := make([]resource.Row, 0, len(endpoints))

	for _, e := range endpoints {
		state, word := e.state()

		rows = append(rows, resource.Row{
			Cells: []string{e.group + "::" + e.module, word, e.count(), e.covers},
			State: state,
			Detail: []resource.Field{
				{Label: "Module", Value: e.group + "::" + e.module},
				{Label: "State", Value: word, Emphasis: state},
				{Label: "Routes", Value: e.count()},
				{Label: "Served", Value: "unknown, not connected", Emphasis: resource.Held},
				{Label: "Covers", Value: e.covers},
			},
		})
	}

	return resource.Listing{
		Sort: "routed, then unwired",
		Columns: []resource.Column{
			{Title: "Module", Width: 26},
			{Title: "State", Width: 11},
			{Title: "Routes", Width: 7, Right: true},
			{Title: "Covers", Flex: true},
		},
		Rows: rows,
	}
}

// routeCounts sums the routes of each state, for the overview.
func routeCounts() (routedN, unwiredN, unportedN int) {
	for _, e := range endpoints {
		switch e.status {
		case routed:
			routedN += e.routes
		case unwired:
			unwiredN += e.routes
		default:
			unportedN++
		}
	}

	return routedN, unwiredN, unportedN
}

func devices() resource.Listing {
	device := func(id, user, name, seen, ip string, verified bool, otks string) resource.Row {
		trust, state := "unverified", resource.Held
		if verified {
			trust, state = "cross-signed", resource.Done
		}

		return resource.Row{
			Cells: []string{id, user, name, trust, seen},
			State: state,
			Detail: []resource.Field{
				{Label: "Device ID", Value: id},
				{Label: "User", Value: user + ":phantom.chat"},
				{Label: "Display name", Value: name},
				{Label: "Trust", Value: trust, Emphasis: state},
				{Label: "Last seen", Value: seen},
				{Label: "Last IP", Value: ip},
				{Label: "One-time keys", Value: otks},
				{Label: "Fallback key", Value: "uploaded"},
				{Label: "Algorithms", Value: "olm.v1, megolm.v1"},
			},
		}
	}

	return resource.Listing{
		Sort: "last seen, newest first",
		Columns: []resource.Column{
			{Title: "Device", Width: 12},
			{Title: "User", Width: 10},
			{Title: "Name", Flex: true},
			{Title: "Trust", Width: 13},
			{Title: "Last seen", Width: 12, Right: true},
		},
		Rows: []resource.Row{
			device("QWERTYUIOP", "@ada", "Element Desktop", "2 min ago", "198.51.100.4", true, "50 of 256"),
			device("ASDFGHJKLZ", "@ada", "Element X on Pixel 9", "1 hour ago", "198.51.100.9", true, "48 of 256"),
			device("ZXCVBNMASD", "@grace", "Fractal", "18 min ago", "203.0.113.42", true, "50 of 256"),
			device("POIUYTREWQ", "@alan", "nheko", "1 hour ago", "192.0.2.77", false, "12 of 256"),
			device("LKJHGFDSAM", "@edsger", "Element Web", "3 hours ago", "192.0.2.15", true, "50 of 256"),
			device("MNBVCXZLKJ", "@ken", "gomuks", "5 hours ago", "198.51.100.61", false, "0 of 256"),
			device("QAZWSXEDCR", "@ada", "Element Web (old)", "34 days ago", "198.51.100.4", false, "50 of 256"),
		},
	}
}

func appservices() resource.Listing {
	bridge := func(id, users, aliases, ping, protocols string, state resource.State) resource.Row {
		return resource.Row{
			Cells: []string{id, users, ping},
			State: state,
			Detail: []resource.Field{
				{Label: "Registration", Value: id},
				{Label: "Users", Value: users},
				{Label: "Aliases", Value: aliases},
				{Label: "Ping", Value: ping, Emphasis: state},
				{Label: "Protocols", Value: protocols},
				{Label: "Rate limited", Value: "no"},
				{Label: "Source", Value: "database"},
			},
		}
	}

	return resource.Listing{
		Sort: "registration ID",
		Columns: []resource.Column{
			{Title: "Registration", Flex: true},
			{Title: "Users", Width: 18},
			{Title: "Ping", Width: 12, Right: true},
		},
		Rows: []resource.Row{
			bridge("discord", "@_discord_.*", "#_discord_.*", "41 ms", "discord", resource.Done),
			bridge("heisenbridge", "@hbirc_.*", "—", "63 ms", "irc", resource.Done),
			bridge("signal", "@signal_.*", "—", "timed out", "signal", resource.Failed),
			bridge("telegram", "@telegram_.*", "#telegram_.*", "88 ms", "telegram", resource.Done),
			bridge("draupnir", "@draupnir", "—", "never", "—", resource.Held),
		},
	}
}

func reports() resource.Listing {
	report := func(at, kind, target, reporter, reason string, state resource.State) resource.Row {
		status := "open"
		switch state {
		case resource.Done:
			status = "resolved"
		case resource.NoState:
			status = "dismissed"
		}

		return resource.Row{
			Cells: []string{at, kind, target, reason},
			State: state,
			Detail: []resource.Field{
				{Label: "Target", Value: target},
				{Label: "Kind", Value: kind},
				{Label: "Status", Value: status, Emphasis: state},
				{Label: "Reported by", Value: reporter + ":phantom.chat"},
				{Label: "Received", Value: "2026-10-03 " + at},
				{Label: "Reason", Value: reason},
			},
		}
	}

	return resource.Listing{
		Sort: "newest first",
		Columns: []resource.Column{
			{Title: "Time", Width: 7},
			{Title: "Kind", Width: 7},
			{Title: "Target", Width: 26},
			{Title: "Reason", Flex: true},
		},
		Rows: []resource.Row{
			report("14:02", "event", "$aT9xQ2:phantom.chat", "@grace", "spam links in #general", resource.Held),
			report("13:30", "user", "@bjarne:phantom.chat", "@alan", "harassment over DM", resource.Held),
			report("11:15", "room", "#crypto-deals:example.net", "@ken", "scam room advertised in #random", resource.Held),
			report("09:48", "event", "$pLm4Kx:example.org", "@ada", "NSFW image without a spoiler", resource.Done),
			report("08:12", "user", "@spambot:example.net", "@linus", "mass invites", resource.Done),
			report("07:01", "event", "$Qw8Ed1:phantom.chat", "@dennis", "accidental report", resource.NoState),
		},
	}
}

func openReports() int {
	n := 0
	for _, r := range reports().Rows {
		if r.State == resource.Held {
			n++
		}
	}

	return n
}
