package app

import (
	"encoding/json"
	"errors"
	"testing"

	tea "charm.land/bubbletea/v2"

	"github.com/HilthonTT/phantom/cli/internal/client"
	"github.com/HilthonTT/phantom/cli/internal/tui/live"
	"github.com/HilthonTT/phantom/cli/internal/tui/modal"
)

const room = "!general:test"

// syncBody is a first sync with one room holding one message from bob.
const syncBody = `{"next_batch":"s1","rooms":{"join":{"!general:test":{
  "timeline":{"events":[
    {"type":"m.room.name","state_key":"","sender":"@alice:test","event_id":"$n","origin_server_ts":1,"content":{"name":"General"}},
    {"type":"m.room.member","state_key":"@alice:test","sender":"@alice:test","event_id":"$a","origin_server_ts":2,"content":{"membership":"join"}},
    {"type":"m.room.message","sender":"@bob:test","event_id":"$1","origin_server_ts":3,"content":{"msgtype":"m.text","body":"hi"}}
  ]}}}}}`

func signedIn(t *testing.T) Model {
	t.Helper()

	m := connected(t, sized(t, 140, 40))
	m, _ = feed(t, m, live.AuthMsg{Session: client.Session{UserID: "@alice:test", AccessToken: "tok"}})

	var resp client.SyncResponse
	if err := json.Unmarshal([]byte(syncBody), &resp); err != nil {
		t.Fatal(err)
	}
	m, _ = feed(t, m, live.SyncedMsg{Gen: m.sync.gen, Resp: resp})

	return m
}

func TestASyncReplacesTheSampleRooms(t *testing.T) {
	m := signedIn(t)

	if !m.chat.Live() || len(m.chat.Channels()) != 1 || m.chat.Channel().Name != "General" {
		t.Fatalf("chat = live %v, %d rooms, open %q", m.chat.Live(), len(m.chat.Channels()), m.chat.Channel().Name)
	}
	if m.sync.since != "s1" {
		t.Errorf("since = %q, want the sync's next_batch", m.sync.since)
	}
}

func TestASyncFromAnEndedLoopIsIgnored(t *testing.T) {
	m := signedIn(t)
	stale := m.sync.gen - 1

	m, _ = feed(t, m, live.SyncedMsg{Gen: stale, Resp: client.SyncResponse{NextBatch: "old"}})
	if m.sync.since != "s1" {
		t.Errorf("since = %q after a stale sync, want s1", m.sync.since)
	}
}

func TestSendingShowsTheMessageAtOnceThenMarksAFailure(t *testing.T) {
	m := signedIn(t)
	m.chatOpen, m.focus = true, focusWorkspace

	m = arrow(t, m, tea.KeyEnter)
	m = press(t, m, "y", "o")
	next, cmd := m.Update(tea.KeyPressMsg{Code: tea.KeyEnter})
	m = next.(Model)
	if cmd == nil {
		t.Fatal("sending started no request")
	}

	msgs := m.chat.Channel().Messages
	last := msgs[len(msgs)-1]
	if last.Body != "yo" || !last.Pending {
		t.Fatalf("last message = %+v, want the pending echo", last)
	}

	m, _ = feed(t, m, live.SentMsg{Gen: m.sync.gen, RoomID: room, TxnID: m.lastTxn(t), Err: errors.New("M_FORBIDDEN")})
	msgs = m.chat.Channel().Messages
	if last := msgs[len(msgs)-1]; !last.Failed {
		t.Errorf("last message = %+v after a refused send, want failed", last)
	}
}

// lastTxn is the transaction ID of the newest message waiting on the server.
func (m Model) lastTxn(t *testing.T) string {
	t.Helper()

	ids := m.sync.rooms.PendingTxns(room)
	if len(ids) == 0 {
		t.Fatal("no message is pending")
	}

	return ids[len(ids)-1]
}

func TestSigningOutPutsTheSampleBack(t *testing.T) {
	m := signedIn(t)

	m, _ = feed(t, m, live.LoggedOutMsg{})
	if m.chat.Live() || m.sync.rooms != nil {
		t.Error("the live rooms survived signing out")
	}
}

func TestARefusedTokenDuringSyncAsksToSignInAgain(t *testing.T) {
	m := signedIn(t)

	err := &client.StatusError{Code: 401, ErrCode: "M_UNKNOWN_TOKEN"}
	m, _ = feed(t, m, live.SyncedMsg{Gen: m.sync.gen, Since: "s1", Err: err})

	if m.modal != modal.Login || m.chat.Live() {
		t.Errorf("modal = %d live = %v, want the login form and the sample rooms", m.modal, m.chat.Live())
	}
}

func TestAFailedSyncIsRetried(t *testing.T) {
	m := signedIn(t)

	m, cmd := feed(t, m, live.SyncedMsg{Gen: m.sync.gen, Since: "s1", Err: errors.New("connection refused")})
	if cmd == nil || !m.chat.Live() {
		t.Error("a failed sync was not retried, or dropped the rooms")
	}
}
