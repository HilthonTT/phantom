package client

import (
	"context"
	"encoding/json"
	"net/http"
	"net/url"
	"strconv"
	"time"
)

// Event is a Matrix event as /sync delivers it, its content left raw for
// whoever knows its type.
type Event struct {
	Type     string  `json:"type"`
	StateKey *string `json:"state_key,omitempty"`
	Sender   string  `json:"sender"`
	EventID  string  `json:"event_id"`
	Redacts  string  `json:"redacts,omitempty"`

	OriginServerTS int64           `json:"origin_server_ts"`
	Content        json.RawMessage `json:"content"`

	Unsigned struct {
		TransactionID   string          `json:"transaction_id,omitempty"`
		RedactedBecause json.RawMessage `json:"redacted_because,omitempty"`
		PrevContent     json.RawMessage `json:"prev_content,omitempty"`
	} `json:"unsigned"`
}

type Events struct {
	Events []Event `json:"events"`
}

type JoinedRoom struct {
	Summary struct {
		Heroes      []string `json:"m.heroes"`
		JoinedCount *int     `json:"m.joined_member_count"`
	} `json:"summary"`

	State       Events `json:"state"`
	Timeline    Events `json:"timeline"`
	Ephemeral   Events `json:"ephemeral"`
	AccountData Events `json:"account_data"`

	UnreadNotifications struct {
		NotificationCount int `json:"notification_count"`
		HighlightCount    int `json:"highlight_count"`
	} `json:"unread_notifications"`
}

type InvitedRoom struct {
	InviteState Events `json:"invite_state"`
}

type SyncResponse struct {
	NextBatch string `json:"next_batch"`

	Rooms struct {
		Join   map[string]JoinedRoom  `json:"join"`
		Invite map[string]InvitedRoom `json:"invite"`
		Leave  map[string]JoinedRoom  `json:"leave"`
	} `json:"rooms"`

	Presence    Events `json:"presence"`
	AccountData Events `json:"account_data"`
}

// TimelineLimit is how many events of each room the first sync brings.
const TimelineLimit = 50

// Sync returns what changed since since, holding the request open up to wait
// for something to happen. An empty since is the first sync, which returns
// every joined room.
func (c *Client) Sync(ctx context.Context, since string, wait time.Duration) (SyncResponse, error) {
	q := url.Values{}
	q.Set("timeout", strconv.FormatInt(wait.Milliseconds(), 10))
	q.Set("filter", `{"room":{"timeline":{"limit":`+strconv.Itoa(TimelineLimit)+`}}}`)
	if since != "" {
		q.Set("since", since)
	}

	var resp SyncResponse
	err := c.doWithin(ctx, wait+requestTimeout*5, http.MethodGet,
		"/_matrix/client/v3/sync?"+q.Encode(), nil, &resp)

	return resp, err
}

// SendText sends a message of msgtype (m.text, m.emote) to a room. txnID
// makes a retried send idempotent, and comes back on the event in /sync, so a
// local echo can be matched to it.
func (c *Client) SendText(ctx context.Context, roomID, txnID, msgtype, body string) (string, error) {
	var resp struct {
		EventID string `json:"event_id"`
	}
	err := c.do(ctx, http.MethodPut, roomPath(roomID)+"/send/m.room.message/"+url.PathEscape(txnID),
		map[string]string{"msgtype": msgtype, "body": body}, &resp)

	return resp.EventID, err
}

// TypingTimeout is how long the server shows the user as typing unless told
// otherwise.
const TypingTimeout = 30 * time.Second

func (c *Client) SetTyping(ctx context.Context, roomID, userID string, typing bool) error {
	body := map[string]any{"typing": typing}
	if typing {
		body["timeout"] = TypingTimeout.Milliseconds()
	}

	return c.do(ctx, http.MethodPut, roomPath(roomID)+"/typing/"+url.PathEscape(userID), body, nil)
}

// MarkRead moves the user's read receipt and fully-read marker to eventID.
func (c *Client) MarkRead(ctx context.Context, roomID, eventID string) error {
	body := map[string]string{"m.fully_read": eventID, "m.read": eventID}

	return c.do(ctx, http.MethodPost, roomPath(roomID)+"/read_markers", body, nil)
}

// Join joins a room by its ID or an alias, returning the room's ID.
func (c *Client) Join(ctx context.Context, roomIDOrAlias string) (string, error) {
	var resp struct {
		RoomID string `json:"room_id"`
	}
	err := c.do(ctx, http.MethodPost, "/_matrix/client/v3/join/"+url.PathEscape(roomIDOrAlias),
		map[string]any{}, &resp)

	return resp.RoomID, err
}

func (c *Client) Leave(ctx context.Context, roomID string) error {
	return c.do(ctx, http.MethodPost, roomPath(roomID)+"/leave", map[string]any{}, nil)
}

func roomPath(roomID string) string {
	return "/_matrix/client/v3/rooms/" + url.PathEscape(roomID)
}
