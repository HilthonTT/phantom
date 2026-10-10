package client

import (
	"context"
	"fmt"
	"net/http"
	"net/url"
	"strings"
)

// The admin API's actions. Each refuses what would lock an admin out: acting
// on the server user, deactivating or demoting oneself, removing the device
// the request came from, or banning or deleting the admin room.

func (c *Client) Deactivate(ctx context.Context, userID string, erase bool) error {
	return c.adminDo(ctx, http.MethodPost, "/users/"+url.PathEscape(userID)+"/deactivate",
		map[string]bool{"erase": erase}, nil)
}

// SetPassword sets a local account's password, signing its devices out when
// logout is set.
func (c *Client) SetPassword(ctx context.Context, userID, password string, logout bool) error {
	return c.adminDo(ctx, http.MethodPut, "/users/"+url.PathEscape(userID)+"/password",
		map[string]any{"password": password, "logout_devices": logout}, nil)
}

func (c *Client) GrantAdmin(ctx context.Context, userID string) error {
	return c.adminDo(ctx, http.MethodPut, "/users/"+url.PathEscape(userID)+"/admin", nil, nil)
}

func (c *Client) RevokeAdmin(ctx context.Context, userID string) error {
	return c.adminDo(ctx, http.MethodDelete, "/users/"+url.PathEscape(userID)+"/admin", nil, nil)
}

func (c *Client) DeleteDevice(ctx context.Context, userID, deviceID string) error {
	return c.adminDo(ctx, http.MethodDelete,
		"/devices/"+url.PathEscape(userID)+"/"+url.PathEscape(deviceID), nil, nil)
}

// NewToken asks for a registration token; nil fields take the server's
// defaults: a random token, unlimited uses, no expiry.
type NewToken struct {
	Token         *string `json:"token,omitempty"`
	UsesAllowed   *int64  `json:"uses_allowed,omitempty"`
	ExpiresInSecs *int64  `json:"expires_in_secs,omitempty"`
}

func (c *Client) CreateToken(ctx context.Context, req NewToken) (RegistrationToken, error) {
	var tok RegistrationToken
	err := c.adminDo(ctx, http.MethodPost, "/registration_tokens", req, &tok)

	return tok, err
}

func (c *Client) RevokeToken(ctx context.Context, token string) error {
	return c.adminDo(ctx, http.MethodDelete, "/registration_tokens/"+url.PathEscape(token), nil, nil)
}

func (c *Client) BanRoom(ctx context.Context, roomID string, banned bool) error {
	method := http.MethodPut
	if !banned {
		method = http.MethodDelete
	}

	return c.adminDo(ctx, method, "/rooms/"+url.PathEscape(roomID)+"/ban", nil, nil)
}

// ShutdownRoom starts evicting a room's local members, answering with the
// task doing it.
func (c *Client) ShutdownRoom(ctx context.Context, roomID string) (string, error) {
	return c.adminTask(ctx, http.MethodPost, "/rooms/"+url.PathEscape(roomID)+"/shutdown", nil)
}

// DeleteRoom starts shutting a room down and purging it, answering with the
// task doing it.
func (c *Client) DeleteRoom(ctx context.Context, roomID string, force bool) (string, error) {
	return c.adminTask(ctx, http.MethodDelete, "/rooms/"+url.PathEscape(roomID),
		map[string]bool{"force": force})
}

func (c *Client) ReloadConfig(ctx context.Context) error {
	return c.adminDo(ctx, http.MethodPost, "/config/reload", nil, nil)
}

// Backup starts a database backup, answering with the task doing it.
func (c *Client) Backup(ctx context.Context) (string, error) {
	return c.adminTask(ctx, http.MethodPost, "/backup", nil)
}

// AdminTask is a long operation the server tracks.
type AdminTask struct {
	ID          string  `json:"id"`
	Action      string  `json:"action"`
	Resource    string  `json:"resource"`
	Status      string  `json:"status"`
	UpdatedAtMs int64   `json:"updated_at_ms"`
	Error       *string `json:"error"`
}

func (c *Client) Tasks(ctx context.Context) ([]AdminTask, error) {
	return adminGet[[]AdminTask](ctx, c, "/tasks")
}

// ResolveAlias finds the room an alias points at.
func (c *Client) ResolveAlias(ctx context.Context, alias string) (string, error) {
	var resp struct {
		RoomID string `json:"room_id"`
	}
	err := c.do(ctx, http.MethodGet, "/_matrix/client/v3/directory/room/"+url.PathEscape(alias), nil, &resp)

	return resp.RoomID, err
}

func (c *Client) adminDo(ctx context.Context, method, path string, in, out any) error {
	return c.doWithin(ctx, adminTimeout, method, adminAPI+path, in, out)
}

func (c *Client) adminTask(ctx context.Context, method, path string, in any) (string, error) {
	var resp struct {
		TaskID string `json:"task_id"`
	}
	err := c.adminDo(ctx, method, path, in, &resp)

	return resp.TaskID, err
}

// DeleteMedia deletes a stored file, by its mxc URI, and its thumbnails.
func (c *Client) DeleteMedia(ctx context.Context, mxc string) error {
	server, id, ok := strings.Cut(strings.TrimPrefix(mxc, "mxc://"), "/")
	if !ok || !strings.HasPrefix(mxc, "mxc://") {
		return fmt.Errorf("%q is not an mxc:// URI", mxc)
	}

	return c.adminDo(ctx, http.MethodDelete, "/media/"+url.PathEscape(server)+"/"+url.PathEscape(id), nil, nil)
}

// PurgeRemoteMedia deletes every copy of another server's media held here,
// answering with how many files went.
func (c *Client) PurgeRemoteMedia(ctx context.Context, server string) (int, error) {
	var resp struct {
		Removed int `json:"removed"`
	}
	err := c.adminDo(ctx, http.MethodDelete, "/federation/"+url.PathEscape(server)+"/media", nil, &resp)

	return resp.Removed, err
}

func (c *Client) DismissReport(ctx context.Context, id string) error {
	return c.adminDo(ctx, http.MethodDelete, "/reports/"+url.PathEscape(id), nil, nil)
}
