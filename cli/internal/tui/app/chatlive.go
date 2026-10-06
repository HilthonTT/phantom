package app

import (
	"fmt"
	"time"

	tea "charm.land/bubbletea/v2"

	"github.com/HilthonTT/phantom/cli/internal/client"
	"github.com/HilthonTT/phantom/cli/internal/tui/live"
	"github.com/HilthonTT/phantom/cli/internal/tui/rooms"
)

// typingRefresh is how often a typing notice is renewed while the user keeps
// writing, inside the server's timeout so the notice does not lapse.
const typingRefresh = client.TypingTimeout * 2 / 3

// chatLive is the live chat's state: the rooms the sync loop has built and
// what has been told to the server about reading and typing in them.
type chatLive struct {
	rooms *rooms.Store

	// gen numbers sync loops; only the current one's results are applied.
	gen   int
	since string

	// read is the event each room's read receipt was last moved to.
	read map[string]string

	typingRoom string
	typingAt   time.Time

	// open is a room just joined, opened once a sync lists it.
	open string

	txns int
}

// startSync begins the sync loop for the signed-in user, ending any loop
// for an earlier one.
func (m Model) startSync(user string) (Model, tea.Cmd) {
	gen := m.sync.gen + 1
	m.sync = chatLive{rooms: rooms.New(user), gen: gen, read: map[string]string{}}
	m.chat.SetStatus("syncing…")

	return m, live.Sync(m.client, gen, "")
}

// stopSync ends the sync loop and puts the sample rooms back.
func (m Model) stopSync() Model {
	m.sync = chatLive{gen: m.sync.gen + 1}
	m.chat.SetSample()
	m.chat.SetStatus("")

	return m
}

func (m Model) synced(msg live.SyncedMsg) (tea.Model, tea.Cmd) {
	if msg.Gen != m.sync.gen || m.sync.rooms == nil {
		return m, nil
	}

	if msg.Err != nil {
		if client.IsUnknownToken(msg.Err) {
			return m.authed(live.AuthMsg{Resumed: true, Refused: true, Err: msg.Err})
		}

		m.chat.SetStatus("sync failed; retrying")
		return m, live.RetrySync(msg.Gen, msg.Since)
	}

	m.sync.rooms.Apply(msg.Resp)
	m.sync.since = msg.Resp.NextBatch
	m.chat.SetStatus("")
	m.refreshChat()

	if m.sync.open != "" && m.chat.Open(m.sync.open) {
		m.sync.open = ""
	}

	next := live.Sync(m.client, m.sync.gen, m.sync.since)
	model, read := m.markRead()

	return model, tea.Batch(next, read)
}

func (m *Model) refreshChat() {
	m.chat.SetLive(m.sync.rooms.Self(), m.sync.rooms.Channels())
}

// markRead moves the read receipt of the open room to its newest event, when
// the chat is on screen and the receipt is not there already.
func (m Model) markRead() (Model, tea.Cmd) {
	if !m.chatOpen || m.sync.rooms == nil || !m.chat.Live() {
		return m, nil
	}

	roomID := m.chat.Channel().ID
	latest := m.sync.rooms.Latest(roomID)
	if roomID == "" || latest == "" || m.sync.read[roomID] == latest {
		return m, nil
	}

	m.sync.read[roomID] = latest
	m.sync.rooms.MarkRead(roomID)
	m.refreshChat()

	return m, live.MarkRead(m.client, roomID, latest)
}

// sendChat posts what the composer held, showing it at once as sending.
func (m Model) sendChat() (Model, tea.Cmd) {
	out, ok := m.chat.Send()
	if !ok {
		return m, nil
	}

	m.sync.txns++
	txn := fmt.Sprintf("phantom.%d.%d", time.Now().UnixMilli(), m.sync.txns)

	m.sync.rooms.Send(out.RoomID, txn, out.Message)
	m.refreshChat()

	model, typing := m.stopTyping()

	return model, tea.Batch(live.Send(m.client, m.sync.gen, out.RoomID, txn, out.Message), typing)
}

func (m Model) sent(msg live.SentMsg) (tea.Model, tea.Cmd) {
	if msg.Gen != m.sync.gen || m.sync.rooms == nil || msg.Err == nil {
		return m, nil
	}

	m.sync.rooms.Fail(msg.RoomID, msg.TxnID)
	m.refreshChat()
	m.chat.SetStatus("not sent: " + msg.Err.Error())

	return m, nil
}

// typed tells the server the user is typing in the open room while the
// composer holds text, renewing the notice as it nears its timeout.
func (m Model) typed() (Model, tea.Cmd) {
	if !m.chat.Live() {
		return m, nil
	}
	if m.chat.Draft() == "" {
		return m.stopTyping()
	}

	roomID := m.chat.Channel().ID
	if roomID == m.sync.typingRoom && time.Since(m.sync.typingAt) < typingRefresh {
		return m, nil
	}

	model, stop := m.stopTyping()
	model.sync.typingRoom, model.sync.typingAt = roomID, time.Now()

	return model, tea.Batch(stop, live.Typing(m.client, roomID, m.chat.Self(), true))
}

func (m Model) stopTyping() (Model, tea.Cmd) {
	roomID := m.sync.typingRoom
	if roomID == "" {
		return m, nil
	}

	m.sync.typingRoom = ""

	return m, live.Typing(m.client, roomID, m.chat.Self(), false)
}

func (m Model) joined(msg live.JoinedMsg) (tea.Model, tea.Cmd) {
	if msg.Err != nil {
		m.chat.SetStatus("could not join " + msg.Target)
		m.ask(noAction, "Could not join "+msg.Target, msg.Err.Error())
		return m, nil
	}

	m.chatOpen, m.focus = true, focusWorkspace
	if !m.chat.Open(msg.RoomID) {
		m.sync.open = msg.RoomID
	}

	return m.markRead()
}

func (m Model) left(msg live.LeftMsg) (tea.Model, tea.Cmd) {
	if msg.Err != nil {
		m.ask(noAction, "Could not leave "+msg.Name, msg.Err.Error())
	}

	return m, nil
}
