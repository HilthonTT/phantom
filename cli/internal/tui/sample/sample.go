// Package sample is the placeholder content the interface is drawn with.
//
// None of it is real: there is no homeserver behind any of these values, and
// nothing here reads a config, opens a socket or touches a database. It exists
// so the layout can be built and looked at before the admin API is written,
// and it is the one package to delete when that happens.
package sample

import "github.com/HilthonTT/phantom/cli/internal/tui/resource"

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
