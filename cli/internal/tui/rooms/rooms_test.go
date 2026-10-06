package rooms

import (
	"encoding/json"
	"testing"

	"github.com/HilthonTT/phantom/cli/internal/client"
	"github.com/HilthonTT/phantom/cli/internal/tui/resource"
)

const self = "@alice:test"

// sync decodes a /sync response body, as the server would send it.
func sync(t *testing.T, body string) client.SyncResponse {
	t.Helper()

	var resp client.SyncResponse
	if err := json.Unmarshal([]byte(body), &resp); err != nil {
		t.Fatal(err)
	}

	return resp
}

const first = `{
  "next_batch": "s1",
  "account_data": {"events": [{"type": "m.direct", "content": {"@bob:test": ["!dm:test"]}}]},
  "presence": {"events": [{"type": "m.presence", "sender": "@bob:test", "content": {"presence": "online"}}]},
  "rooms": {"join": {
    "!general:test": {
      "state": {"events": [
        {"type": "m.room.name", "state_key": "", "sender": "@alice:test", "event_id": "$n", "content": {"name": "General"}},
        {"type": "m.room.topic", "state_key": "", "sender": "@alice:test", "event_id": "$t", "content": {"topic": "Say hi"}},
        {"type": "m.room.encryption", "state_key": "", "sender": "@alice:test", "event_id": "$e", "content": {"algorithm": "m.megolm.v1.aes-sha2"}},
        {"type": "m.room.power_levels", "state_key": "", "sender": "@alice:test", "event_id": "$p", "content": {"users": {"@alice:test": 100}}},
        {"type": "m.room.member", "state_key": "@alice:test", "sender": "@alice:test", "event_id": "$ma", "content": {"membership": "join", "displayname": "Alice"}}
      ]},
      "timeline": {"events": [
        {"type": "m.room.member", "state_key": "@bob:test", "sender": "@bob:test", "event_id": "$mb", "origin_server_ts": 1000, "content": {"membership": "join", "displayname": "Bob"}},
        {"type": "m.room.message", "sender": "@bob:test", "event_id": "$1", "origin_server_ts": 2000, "content": {"msgtype": "m.text", "body": "helo"}},
        {"type": "m.room.message", "sender": "@bob:test", "event_id": "$2", "origin_server_ts": 3000, "content": {"msgtype": "m.text", "body": "* hello", "m.new_content": {"msgtype": "m.text", "body": "hello"}, "m.relates_to": {"rel_type": "m.replace", "event_id": "$1"}}},
        {"type": "m.reaction", "sender": "@alice:test", "event_id": "$3", "origin_server_ts": 4000, "content": {"m.relates_to": {"rel_type": "m.annotation", "event_id": "$1", "key": "👍"}}},
        {"type": "m.room.message", "sender": "@alice:test", "event_id": "$4", "origin_server_ts": 5000, "content": {"msgtype": "m.text", "body": "in a thread", "m.relates_to": {"rel_type": "m.thread", "event_id": "$1"}}},
        {"type": "m.room.message", "sender": "@bob:test", "event_id": "$5", "origin_server_ts": 6000, "content": {"msgtype": "m.emote", "body": "waves"}},
        {"type": "m.room.message", "sender": "@bob:test", "event_id": "$6", "origin_server_ts": 7000, "content": {"msgtype": "m.text", "body": "oops"}},
        {"type": "m.room.redaction", "sender": "@bob:test", "event_id": "$7", "origin_server_ts": 8000, "redacts": "$6", "content": {}}
      ]},
      "ephemeral": {"events": [
        {"type": "m.typing", "content": {"user_ids": ["@bob:test"]}},
        {"type": "m.receipt", "content": {"$7": {"m.read": {"@bob:test": {"ts": 9000}}}}}
      ]},
      "unread_notifications": {"notification_count": 3}
    },
    "!dm:test": {
      "state": {"events": [
        {"type": "m.room.member", "state_key": "@alice:test", "sender": "@alice:test", "event_id": "$da", "content": {"membership": "join"}},
        {"type": "m.room.member", "state_key": "@bob:test", "sender": "@bob:test", "event_id": "$db", "content": {"membership": "join", "displayname": "Bob"}}
      ]},
      "timeline": {"events": [
        {"type": "m.room.message", "sender": "@bob:test", "event_id": "$d1", "origin_server_ts": 9500, "content": {"msgtype": "m.text", "body": "psst"}}
      ]}
    }
  }}
}`

func channel(t *testing.T, s *Store, id string) resource.Channel {
	t.Helper()

	for _, ch := range s.Channels() {
		if ch.ID == id {
			return ch
		}
	}
	t.Fatalf("no channel %s", id)

	return resource.Channel{}
}

func TestARoomAsTheChatShowsIt(t *testing.T) {
	s := New(self)
	s.Apply(sync(t, first))

	ch := channel(t, s, "!general:test")
	if ch.Name != "General" || ch.Topic != "Say hi" || !ch.Encrypted || ch.Direct || ch.Unread != 3 {
		t.Errorf("room = %q %q encrypted=%v direct=%v unread=%d", ch.Name, ch.Topic, ch.Encrypted, ch.Direct, ch.Unread)
	}

	if len(ch.Members) != 2 || ch.Members[0].Name != "Alice" || !ch.Members[0].Admin ||
		ch.Members[1].Presence != resource.Online || ch.Members[1].Admin {
		t.Errorf("members = %+v", ch.Members)
	}

	if len(ch.Typing) != 1 || ch.Typing[0] != "@bob:test" {
		t.Errorf("typing = %v", ch.Typing)
	}
	if len(ch.ReadBy) != 1 || ch.ReadBy[0] != "@bob:test" {
		t.Errorf("read by = %v, want bob, whose receipt is on the newest event", ch.ReadBy)
	}

	want := []resource.Message{
		{Sender: "@bob:test", Kind: resource.Membership, Body: "joined the room"},
		{Sender: "@bob:test", Body: "hello", Edited: true, Replies: 1,
			Reactions: []resource.Reaction{{Key: "👍", Count: 1, Mine: true}}},
		{Sender: "@bob:test", Kind: resource.Emote, Body: "waves"},
		{Sender: "@bob:test", Redacted: true},
	}
	if len(ch.Messages) != len(want) {
		t.Fatalf("messages = %+v, want %d of them", ch.Messages, len(want))
	}
	for i, w := range want {
		got := ch.Messages[i]
		if got.Sender != w.Sender || got.Kind != w.Kind || got.Body != w.Body || got.Edited != w.Edited ||
			got.Redacted != w.Redacted || got.Replies != w.Replies || len(got.Reactions) != len(w.Reactions) {
			t.Errorf("message %d = %+v, want %+v", i, got, w)
		}
		for j, r := range w.Reactions {
			if got.Reactions[j] != r {
				t.Errorf("message %d reaction %d = %+v, want %+v", i, j, got.Reactions[j], r)
			}
		}
	}
}

func TestDirectMessagesComeAfterRoomsAndAreNamedForThePeer(t *testing.T) {
	s := New(self)
	s.Apply(sync(t, first))

	chs := s.Channels()
	if len(chs) != 2 || chs[0].ID != "!general:test" || chs[1].ID != "!dm:test" {
		t.Fatalf("order = %v, want the room before the DM", chs)
	}
	if !chs[1].Direct || chs[1].Name != "Bob" {
		t.Errorf("DM = %q direct=%v, want Bob", chs[1].Name, chs[1].Direct)
	}
}

func TestALocalEchoIsReplacedByItsEvent(t *testing.T) {
	s := New(self)
	s.Apply(sync(t, first))

	s.Send("!dm:test", "txn1", resource.Message{Sender: self, Body: "hi bob"})
	msgs := channel(t, s, "!dm:test").Messages
	if last := msgs[len(msgs)-1]; !last.Pending || last.Body != "hi bob" {
		t.Fatalf("last message = %+v, want the pending echo", last)
	}

	s.Apply(sync(t, `{"next_batch":"s2","rooms":{"join":{"!dm:test":{"timeline":{"events":[
	  {"type":"m.room.message","sender":"@alice:test","event_id":"$d2","origin_server_ts":9600,
	   "content":{"msgtype":"m.text","body":"hi bob"},"unsigned":{"transaction_id":"txn1"}}]}}}}}`))

	msgs = channel(t, s, "!dm:test").Messages
	if len(msgs) != 2 || msgs[1].Pending || msgs[1].EventID != "$d2" {
		t.Errorf("messages = %+v, want the echo replaced by $d2", msgs)
	}
}

func TestAFailedSendStaysVisible(t *testing.T) {
	s := New(self)
	s.Apply(sync(t, first))

	s.Send("!dm:test", "txn1", resource.Message{Sender: self, Body: "hi"})
	s.Fail("!dm:test", "txn1")

	msgs := channel(t, s, "!dm:test").Messages
	if last := msgs[len(msgs)-1]; !last.Failed || last.Pending {
		t.Errorf("last = %+v, want failed", last)
	}
}

func TestLeavingDropsTheRoom(t *testing.T) {
	s := New(self)
	s.Apply(sync(t, first))
	s.Apply(sync(t, `{"next_batch":"s2","rooms":{"leave":{"!dm:test":{}}}}`))

	for _, ch := range s.Channels() {
		if ch.ID == "!dm:test" {
			t.Fatal("the room left is still listed")
		}
	}
}

func TestMembershipChangesAreDescribed(t *testing.T) {
	s := New(self)
	s.Apply(sync(t, `{"next_batch":"s1","rooms":{"join":{"!r:test":{"timeline":{"events":[
	  {"type":"m.room.member","state_key":"@carol:test","sender":"@alice:test","event_id":"$a","origin_server_ts":1,"content":{"membership":"invite"}},
	  {"type":"m.room.member","state_key":"@carol:test","sender":"@carol:test","event_id":"$b","origin_server_ts":2,"content":{"membership":"join","displayname":"Carol"},"unsigned":{"prev_content":{"membership":"invite"}}},
	  {"type":"m.room.member","state_key":"@carol:test","sender":"@carol:test","event_id":"$c","origin_server_ts":3,"content":{"membership":"join","displayname":"Caz"},"unsigned":{"prev_content":{"membership":"join","displayname":"Carol"}}},
	  {"type":"m.room.member","state_key":"@carol:test","sender":"@alice:test","event_id":"$d","origin_server_ts":4,"content":{"membership":"ban","reason":"spam"},"unsigned":{"prev_content":{"membership":"join"}}}
	]}}}}}`))

	want := []string{"was invited by alice", "joined the room", "is now known as Caz", "was banned by alice: spam"}
	msgs := channel(t, s, "!r:test").Messages
	if len(msgs) != len(want) {
		t.Fatalf("messages = %+v", msgs)
	}
	for i, w := range want {
		if msgs[i].Body != w || msgs[i].Sender != "@carol:test" {
			t.Errorf("message %d = %q from %s, want %q about carol", i, msgs[i].Body, msgs[i].Sender, w)
		}
	}
}
