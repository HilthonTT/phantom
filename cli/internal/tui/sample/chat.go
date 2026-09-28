package sample

import "github.com/HilthonTT/phantom/cli/internal/tui/resource"

const Self = "@admin:phantom.chat"

var (
	admin = resource.Member{ID: Self, Name: "admin", Presence: resource.Online, Admin: true}
	alice = resource.Member{ID: "@alice:phantom.chat", Name: "Alice", Presence: resource.Online, Admin: true}
	bob   = resource.Member{ID: "@bob:phantom.chat", Name: "Bob", Presence: resource.Idle}
	carol = resource.Member{ID: "@carol:example.org", Name: "Carol", Presence: resource.Online}
	dave  = resource.Member{ID: "@dave:phantom.chat", Name: "Dave", Presence: resource.Offline}
	erin  = resource.Member{ID: "@erin:matrix.example.org", Name: "Erin", Presence: resource.Idle}
	bot   = resource.Member{ID: "@phantom:phantom.chat", Name: "phantom", Presence: resource.Online, Admin: true}
)

func Channels() []resource.Channel {
	return []resource.Channel{
		{
			Name:      "#general",
			Topic:     "Everything phantom.chat — be kind, stay on topic-ish",
			Encrypted: true,
			Members:   []resource.Member{admin, alice, bob, carol, dave, erin},
			Messages: []resource.Message{
				msg("09:02", alice.ID, "morning all"),
				msg("09:03", bob.ID, "morning! did the upgrade to 0.1.0 go through last night?"),
				msg("09:05", Self, "it did, the migrations ran in about four minutes"),
				msg("09:05", Self, "the database shrank by a few hundred megabytes too, the state compressor finally caught up"),
				join("09:11", carol.ID),
				msg("09:12", carol.ID, "hi from example.org — federation looks much snappier today"),
				msg("09:14", alice.ID, "that'll be the new resolver cache, server names are only looked up once an hour now"),
				emote("09:15", bob.ID, "puts the kettle on"),
				msg("09:21", erin.ID, "is there a known issue with media thumbnails? a couple of mine come back as 404"),
				msg("09:23", Self, "not known, can you send me the mxc URI? I'll look at the media store"),
				msg("09:24", erin.ID, "sent it in a DM"),
				msg("09:31", dave.ID, "reminder that the purge on #general is running, history before March will go"),
				msg("09:32", alice.ID, "good, it was getting slow to backfill"),
			},
		},
		{
			Name:      "#announcements",
			Topic:     "Server news, maintenance windows and releases",
			Encrypted: true,
			Unread:    2,
			Members:   []resource.Member{admin, alice, bot},
			Messages: []resource.Message{
				notice("08:00", bot.ID, "Nightly backup is waiting for the write lock."),
				msg("08:30", Self, "Planned maintenance on Thursday 22:00 UTC, expect about ten minutes of downtime."),
				notice("09:00", bot.ID, "phantom 0.1.0 is now running on phantom.chat."),
			},
		},
		{
			Name:      "#dev",
			Topic:     "Hacking on phantom itself",
			Encrypted: true,
			Unread:    5,
			Members:   []resource.Member{admin, alice, bob},
			Messages: []resource.Message{
				msg("10:02", bob.ID, "the federation send queue retries forever when a server is gone"),
				msg("10:03", alice.ID, "there's a backoff, it just caps at 24 hours"),
				msg("10:05", bob.ID, "ah, I missed that"),
				msg("10:09", alice.ID, "PR for the TUI chat panel is up, reviews welcome"),
				emote("10:10", bob.ID, "takes a look"),
			},
		},
		{
			Name:      "#ops",
			Topic:     "Alerts and on-call",
			Encrypted: true,
			Members:   []resource.Member{admin, dave, bot},
			Messages: []resource.Message{
				notice("14:10", bot.ID, "federation send to example.org timed out"),
				msg("14:12", dave.ID, "looking into it"),
				msg("14:20", dave.ID, "their side was restarting, all green again"),
			},
		},
		{
			Name:    "#support",
			Topic:   "Ask for help with your account",
			Unread:  1,
			Members: []resource.Member{admin, carol, erin},
			Messages: []resource.Message{
				msg("11:40", carol.ID, "how do I reset my password if I lost my email access?"),
			},
		},
		{
			Name:      "Alice",
			Direct:    true,
			Encrypted: true,
			Members:   []resource.Member{admin, alice},
			Messages: []resource.Message{
				msg("08:50", alice.ID, "can you give me moderator rights in #random?"),
				msg("08:52", Self, "done"),
				msg("08:52", alice.ID, "thanks!"),
			},
		},
		{
			Name:      "Erin",
			Direct:    true,
			Encrypted: true,
			Unread:    1,
			Members:   []resource.Member{admin, erin},
			Messages: []resource.Message{
				msg("09:24", erin.ID, "mxc://phantom.chat/aBcDeFgHiJkLmNoP — the thumbnail 404s, the original loads fine"),
			},
		},
	}
}

func msg(time, sender, body string) resource.Message {
	return resource.Message{Time: time, Sender: sender, Body: body}
}

func emote(time, sender, body string) resource.Message {
	return resource.Message{Time: time, Sender: sender, Body: body, Kind: resource.Emote}
}

func notice(time, sender, body string) resource.Message {
	return resource.Message{Time: time, Sender: sender, Body: body, Kind: resource.Notice}
}

func join(time, sender string) resource.Message {
	return resource.Message{Time: time, Sender: sender, Body: "joined the room", Kind: resource.Membership}
}
