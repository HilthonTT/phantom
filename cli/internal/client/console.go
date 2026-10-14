package client

import (
	"context"
	"net/http"
	"net/url"
	"strconv"
)

// The admin API's routes for looking into the server rather than acting on
// it: the route table, the log as it grows, a room's insides and the audit
// log.

// Route is one method on one path the server serves.
type Route struct {
	Method string `json:"method"`
	Path   string `json:"path"`

	// Group is client, federation, media, admin, oidc or well-known.
	Group string `json:"group"`

	// Endpoint is the ruma request the route implements, when it has one.
	Endpoint *string `json:"endpoint"`
	Stable   bool    `json:"stable"`
}

func (c *Client) Routes(ctx context.Context) ([]Route, error) {
	return adminGet[[]Route](ctx, c, "/routes")
}

// LogsSince are the lines after the one numbered since, newest first.
func (c *Client) LogsSince(ctx context.Context, since int64, limit int) ([]LogLine, error) {
	return adminGet[[]LogLine](ctx, c,
		"/logs?limit="+strconv.Itoa(limit)+"&since="+strconv.FormatInt(since, 10))
}

// LogCapture is the lowest level the server's log keeps: info or debug.
func (c *Client) LogCapture(ctx context.Context) (string, error) {
	resp, err := adminGet[struct {
		Level string `json:"level"`
	}](ctx, c, "/logs/capture")

	return resp.Level, err
}

func (c *Client) SetLogCapture(ctx context.Context, level string) error {
	return c.adminDo(ctx, http.MethodPut, "/logs/capture", map[string]string{"level": level}, nil)
}

// StateEvent is one event of a room's current state.
type StateEvent struct {
	EventID        string `json:"event_id"`
	Type           string `json:"type"`
	StateKey       string `json:"state_key"`
	Sender         string `json:"sender"`
	OriginServerTs int64  `json:"origin_server_ts"`

	// Content is the event's content as stored, for showing as JSON.
	Content RawJSON `json:"content"`
}

// TimelineEvent is one event of a room's timeline, with its place in the
// room's graph.
type TimelineEvent struct {
	EventID        string   `json:"event_id"`
	Type           string   `json:"type"`
	StateKey       *string  `json:"state_key"`
	Sender         string   `json:"sender"`
	OriginServerTs int64    `json:"origin_server_ts"`
	Depth          int64    `json:"depth"`
	PrevEvents     []string `json:"prev_events"`
	Content        RawJSON  `json:"content"`
}

// TimelinePage is a page of events, newest first; Next, when set, asks for
// the older ones after it.
type TimelinePage struct {
	Events []TimelineEvent `json:"events"`
	Next   *string         `json:"next"`
}

// Extremity is one of a room's forward extremities; the event's fields are
// nil when the server lists it without holding it.
type Extremity struct {
	EventID        string  `json:"event_id"`
	Depth          *int64  `json:"depth"`
	Sender         *string `json:"sender"`
	OriginServerTs *int64  `json:"origin_server_ts"`
}

// RawJSON keeps a JSON value as it came, for printing.
type RawJSON []byte

func (r *RawJSON) UnmarshalJSON(b []byte) error {
	*r = append((*r)[:0], b...)
	return nil
}

func (r RawJSON) MarshalJSON() ([]byte, error) {
	if len(r) == 0 {
		return []byte("null"), nil
	}

	return r, nil
}

func adminRoomPath(roomID, rest string) string { return "/rooms/" + url.PathEscape(roomID) + rest }

func (c *Client) RoomState(ctx context.Context, roomID string) ([]StateEvent, error) {
	return adminGet[[]StateEvent](ctx, c, adminRoomPath(roomID, "/state"))
}

// RoomEvents is a page of a room's timeline, older than from when it is
// given.
func (c *Client) RoomEvents(ctx context.Context, roomID, from string, limit int) (TimelinePage, error) {
	q := url.Values{"limit": {strconv.Itoa(limit)}}
	if from != "" {
		q.Set("from", from)
	}

	return adminGet[TimelinePage](ctx, c, adminRoomPath(roomID, "/events?"+q.Encode()))
}

func (c *Client) RoomExtremities(ctx context.Context, roomID string) ([]Extremity, error) {
	return adminGet[[]Extremity](ctx, c, adminRoomPath(roomID, "/extremities"))
}

func (c *Client) RoomServers(ctx context.Context, roomID string) ([]string, error) {
	return adminGet[[]string](ctx, c, adminRoomPath(roomID, "/servers"))
}

// AuditRecord is one admin action and how it went.
type AuditRecord struct {
	ID     string `json:"id"`
	AtMs   int64  `json:"at_ms"`
	Admin  string `json:"admin"`
	Via    string `json:"via"`
	Action string `json:"action"`
	Target string `json:"target"`
	OK     bool   `json:"ok"`
	Result string `json:"result"`
}

// AuditPage is a page of the audit log, newest first; Next, when set, asks
// for the older records after it.
type AuditPage struct {
	Records []AuditRecord `json:"records"`
	Next    *string       `json:"next"`
}

// Audit is a page of the audit log, older than before when it is given.
func (c *Client) Audit(ctx context.Context, before string, limit int) (AuditPage, error) {
	q := url.Values{"limit": {strconv.Itoa(limit)}}
	if before != "" {
		q.Set("before", before)
	}

	return adminGet[AuditPage](ctx, c, "/audit?"+q.Encode())
}
