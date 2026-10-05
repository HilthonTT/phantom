package client

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
)

// DeviceName is the device name the console logs in with, so it is easy to
// find, and sign out, in another client's session list.
const DeviceName = "phantom console"

// Session is a logged-in device on the server.
type Session struct {
	UserID      string `json:"user_id"`
	DeviceID    string `json:"device_id"`
	AccessToken string `json:"access_token"`
}

// ErrNotAdmin is the admin API refusing a token whose user is not an admin.
var ErrNotAdmin = errors.New("this account is not an admin of the server")

// Login signs in with a password and keeps the session for the requests
// after it.
func (c *Client) Login(ctx context.Context, user, password string) (Session, error) {
	body := map[string]any{
		"type":                        "m.login.password",
		"identifier":                  map[string]string{"type": "m.id.user", "user": user},
		"password":                    password,
		"initial_device_display_name": DeviceName,
	}

	var s Session
	if err := c.do(ctx, http.MethodPost, "/_matrix/client/v3/login", body, &s); err != nil {
		return Session{}, err
	}

	c.Resume(s)

	return s, nil
}

// Resume uses a session saved from an earlier login.
func (c *Client) Resume(s Session) {
	c.mu.Lock()
	defer c.mu.Unlock()

	c.session = &s
}

// Session is the session requests are signed with, if any.
func (c *Client) Session() (Session, bool) {
	c.mu.RLock()
	defer c.mu.RUnlock()

	if c.session == nil {
		return Session{}, false
	}

	return *c.session, true
}

// Logout ends the session on the server. The local session is dropped even
// when the server cannot be reached, since the token is then useless here.
func (c *Client) Logout(ctx context.Context) error {
	if _, ok := c.Session(); !ok {
		return nil
	}

	err := c.do(ctx, http.MethodPost, "/_matrix/client/v3/logout", map[string]any{}, nil)

	c.mu.Lock()
	c.session = nil
	c.mu.Unlock()

	return err
}

// Admin is who the admin API says the session belongs to.
type Admin struct {
	UserID     string `json:"user_id"`
	DeviceID   string `json:"device_id"`
	ServerName string `json:"server_name"`
}

// WhoAmI asks the admin API who the session is, failing with ErrNotAdmin
// when the user is not an admin.
func (c *Client) WhoAmI(ctx context.Context) (Admin, error) {
	var a Admin
	err := c.do(ctx, http.MethodGet, "/_phantom/admin/v1/whoami", nil, &a)

	if se, ok := errors.AsType[*StatusError](err); ok && se.ErrCode == "M_FORBIDDEN" {
		return Admin{}, ErrNotAdmin
	}

	return a, err
}

// do sends a request signed with the session, if any, with in as its JSON
// body unless nil, and decodes a 200's body into out unless nil.
func (c *Client) do(ctx context.Context, method, path string, in, out any) error {
	ctx, cancel := context.WithTimeout(ctx, requestTimeout)
	defer cancel()

	var body io.Reader = http.NoBody
	if in != nil {
		raw, err := json.Marshal(in)
		if err != nil {
			return err
		}
		body = bytes.NewReader(raw)
	}

	req, err := http.NewRequestWithContext(ctx, method, c.base.String()+path, body)
	if err != nil {
		return err
	}

	req.Header.Set("Accept", "application/json")
	if in != nil {
		req.Header.Set("Content-Type", "application/json")
	}
	if s, ok := c.Session(); ok {
		req.Header.Set("Authorization", "Bearer "+s.AccessToken)
	}

	resp, err := c.http.Do(req)
	if err != nil {
		return unwrapURLError(err)
	}
	defer func() { _ = resp.Body.Close() }()

	if resp.StatusCode != http.StatusOK {
		return statusError(path, resp)
	}

	if out == nil {
		return nil
	}
	if err := json.NewDecoder(resp.Body).Decode(out); err != nil {
		return fmt.Errorf("%s: %w", path, err)
	}

	return nil
}

// Drop forgets the session without telling the server, for a token the
// server has already refused.
func (c *Client) Drop() {
	c.mu.Lock()
	defer c.mu.Unlock()

	c.session = nil
}

// IsUnknownToken is whether err is the server refusing the session's token,
// so signing in again is the only way on.
func IsUnknownToken(err error) bool {
	se, ok := errors.AsType[*StatusError](err)
	return ok && (se.ErrCode == "M_UNKNOWN_TOKEN" || se.ErrCode == "M_MISSING_TOKEN")
}
