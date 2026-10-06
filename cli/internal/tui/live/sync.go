package live

import (
	"context"
	"time"

	tea "charm.land/bubbletea/v2"

	"github.com/HilthonTT/phantom/cli/internal/client"
	"github.com/HilthonTT/phantom/cli/internal/tui/resource"
)

// SyncWait is how long a sync waits for something to happen.
const SyncWait = 30 * time.Second

// SyncRetry is how long a failed sync waits before trying again.
const SyncRetry = 5 * time.Second

// SyncedMsg is a sync finishing. Gen names the sync loop that started it, so a
// loop ended by a sign-out cannot deliver into the next account's rooms.
type SyncedMsg struct {
	Gen   int
	Since string

	Resp client.SyncResponse
	Err  error
}

// SyncRetryMsg is the wait after a failed sync ending.
type SyncRetryMsg struct {
	Gen   int
	Since string
}

// Sync asks for what changed since since; the first sync, with since empty,
// returns at once with every room.
func Sync(c *client.Client, gen int, since string) tea.Cmd {
	wait := SyncWait
	if since == "" {
		wait = 0
	}

	return func() tea.Msg {
		resp, err := c.Sync(context.Background(), since, wait)
		return SyncedMsg{Gen: gen, Since: since, Resp: resp, Err: err}
	}
}

func RetrySync(gen int, since string) tea.Cmd {
	return tea.Tick(SyncRetry, func(time.Time) tea.Msg { return SyncRetryMsg{Gen: gen, Since: since} })
}

// SentMsg is a message send finishing.
type SentMsg struct {
	Gen    int
	RoomID string
	TxnID  string
	Err    error
}

// Send posts a message written in the chat.
func Send(c *client.Client, gen int, roomID, txnID string, msg resource.Message) tea.Cmd {
	msgtype := "m.text"
	if msg.Kind == resource.Emote {
		msgtype = "m.emote"
	}

	return func() tea.Msg {
		_, err := c.SendText(context.Background(), roomID, txnID, msgtype, msg.Body)
		return SentMsg{Gen: gen, RoomID: roomID, TxnID: txnID, Err: err}
	}
}

// ChatErrMsg is a chat request that failed in the background: a typing notice
// or read receipt, which nothing waits on.
type ChatErrMsg struct{ Err error }

func quietly(f func(context.Context) error) tea.Cmd {
	return func() tea.Msg {
		if err := f(context.Background()); err != nil {
			return ChatErrMsg{Err: err}
		}
		return nil
	}
}

func Typing(c *client.Client, roomID, userID string, typing bool) tea.Cmd {
	return quietly(func(ctx context.Context) error { return c.SetTyping(ctx, roomID, userID, typing) })
}

func MarkRead(c *client.Client, roomID, eventID string) tea.Cmd {
	return quietly(func(ctx context.Context) error { return c.MarkRead(ctx, roomID, eventID) })
}

// JoinedMsg is a join finishing, with the room's ID.
type JoinedMsg struct {
	Target string
	RoomID string
	Err    error
}

func Join(c *client.Client, target string) tea.Cmd {
	return func() tea.Msg {
		id, err := c.Join(context.Background(), target)
		return JoinedMsg{Target: target, RoomID: id, Err: err}
	}
}

// LeftMsg is leaving a room finishing.
type LeftMsg struct {
	RoomID string
	Name   string
	Err    error
}

func Leave(c *client.Client, roomID, name string) tea.Cmd {
	return func() tea.Msg {
		return LeftMsg{RoomID: roomID, Name: name, Err: c.Leave(context.Background(), roomID)}
	}
}
