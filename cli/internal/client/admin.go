package client

import (
	"context"
	"net/http"
	"time"
)

// The admin API's answers, field for field. Times are milliseconds since the
// Unix epoch, absent when unknown.

type Stats struct {
	ServerName string `json:"server_name"`
	Version    string `json:"version"`

	StartedAtMs int64 `json:"started_at_ms"`
	UptimeSecs  int64 `json:"uptime_secs"`

	LocalUsers       int `json:"local_users"`
	ActiveLocalUsers int `json:"active_local_users"`
	Rooms            int `json:"rooms"`
	Appservices      int `json:"appservices"`

	DatabaseBytes int64 `json:"database_bytes"`

	Federation        bool `json:"federation"`
	Registration      bool `json:"registration"`
	RegistrationToken bool `json:"registration_token"`
	ReadOnly          bool `json:"read_only"`
}

type AdminUser struct {
	UserID      string  `json:"user_id"`
	DisplayName *string `json:"display_name"`

	Admin       bool `json:"admin"`
	Deactivated bool `json:"deactivated"`
	ServerUser  bool `json:"server_user"`

	Devices     int    `json:"devices"`
	RoomsJoined int    `json:"rooms_joined"`
	LastSeenMs  *int64 `json:"last_seen_ms"`
}

type AdminDevice struct {
	UserID      string  `json:"user_id"`
	DeviceID    string  `json:"device_id"`
	DisplayName *string `json:"display_name"`
	LastSeenIP  *string `json:"last_seen_ip"`
	LastSeenMs  *int64  `json:"last_seen_ms"`
}

type RegistrationToken struct {
	Token       string `json:"token"`
	Source      string `json:"source"`
	Uses        *int64 `json:"uses"`
	MaxUses     *int64 `json:"max_uses"`
	ExpiresAtMs *int64 `json:"expires_at_ms"`
}

type AdminRoom struct {
	RoomID         string  `json:"room_id"`
	Name           *string `json:"name"`
	CanonicalAlias *string `json:"canonical_alias"`
	Topic          *string `json:"topic"`
	Version        *string `json:"version"`

	JoinedMembers int `json:"joined_members"`
	LocalMembers  int `json:"local_members"`

	Encrypted bool   `json:"encrypted"`
	JoinRule  string `json:"join_rule"`
	Published bool   `json:"published"`
	Banned    bool   `json:"banned"`
	Disabled  bool   `json:"disabled"`
}

type Appservice struct {
	ID              string  `json:"id"`
	URL             *string `json:"url"`
	SenderLocalpart string  `json:"sender_localpart"`

	Users   []string `json:"users"`
	Aliases []string `json:"aliases"`
	Rooms   []string `json:"rooms"`

	RateLimited bool     `json:"rate_limited"`
	Protocols   []string `json:"protocols"`
}

type Setting struct {
	Key   string `json:"key"`
	Value string `json:"value"`
}

const adminAPI = "/_phantom/admin/v1"

// adminTimeout bounds an admin listing, which walks a whole table and so can
// take far longer than a client request on a large server.
const adminTimeout = 15 * time.Second

func adminGet[T any](ctx context.Context, c *Client, path string) (T, error) {
	var out T
	err := c.doWithin(ctx, adminTimeout, http.MethodGet, adminAPI+path, nil, &out)

	return out, err
}

func (c *Client) Stats(ctx context.Context) (Stats, error) {
	return adminGet[Stats](ctx, c, "/stats")
}

func (c *Client) Users(ctx context.Context) ([]AdminUser, error) {
	return adminGet[[]AdminUser](ctx, c, "/users")
}

func (c *Client) Devices(ctx context.Context) ([]AdminDevice, error) {
	return adminGet[[]AdminDevice](ctx, c, "/devices")
}

func (c *Client) RegistrationTokens(ctx context.Context) ([]RegistrationToken, error) {
	return adminGet[[]RegistrationToken](ctx, c, "/registration_tokens")
}

func (c *Client) Rooms(ctx context.Context) ([]AdminRoom, error) {
	return adminGet[[]AdminRoom](ctx, c, "/rooms")
}

func (c *Client) Appservices(ctx context.Context) ([]Appservice, error) {
	return adminGet[[]Appservice](ctx, c, "/appservices")
}

func (c *Client) Settings(ctx context.Context) ([]Setting, error) {
	return adminGet[[]Setting](ctx, c, "/settings")
}
