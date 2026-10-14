package client

import (
	"context"
	"fmt"
	"net/http"
	"net/url"
	"strings"
)

// The admin API's moderation actions: the runtime server block list, media
// quarantine, and purging old history and media.

// BlockServer adds a server, that host exactly, to the runtime block list.
func (c *Client) BlockServer(ctx context.Context, server string) error {
	return c.adminDo(ctx, http.MethodPost, "/federation/blocks", map[string]string{"server": server}, nil)
}

// UnblockServer removes every runtime block pattern matching a server. A
// server the config file blocks stays blocked, and the answer says by what.
func (c *Client) UnblockServer(ctx context.Context, server string) (stillBlockedBy []string, err error) {
	var resp struct {
		StillBlockedBy []string `json:"still_blocked_by"`
	}
	err = c.adminDo(ctx, http.MethodDelete, "/federation/blocks", map[string]string{"server": server}, &resp)

	return resp.StillBlockedBy, err
}

// QuarantineMedia keeps a file but answers it as missing, and stops it ever
// being fetched again; UnquarantineMedia lifts that.
func (c *Client) QuarantineMedia(ctx context.Context, mxc string, quarantined bool) error {
	path, err := mxcPath(mxc)
	if err != nil {
		return err
	}

	method := http.MethodPut
	if !quarantined {
		method = http.MethodDelete
	}

	return c.adminDo(ctx, method, path+"/quarantine", nil, nil)
}

// PurgeHistory deletes a room's timeline events sent before a time, keeping
// its state and its local users' events, as a task; it answers with the
// task's ID.
func (c *Client) PurgeHistory(ctx context.Context, roomID string, beforeMs int64) (string, error) {
	return c.adminTask(ctx, http.MethodPost, "/rooms/"+url.PathEscape(roomID)+"/purge_history",
		map[string]int64{"before_ts_ms": beforeMs})
}

// PurgeOldMedia deletes the stored files of an origin, "remote" or "local",
// older than a number of days, as a task; it answers with the task's ID.
func (c *Client) PurgeOldMedia(ctx context.Context, origin string, days int64) (string, error) {
	return c.adminTask(ctx, http.MethodPost, "/media/purge",
		map[string]any{"origin": origin, "older_than_days": days})
}

func mxcPath(mxc string) (string, error) {
	server, id, ok := strings.Cut(strings.TrimPrefix(mxc, "mxc://"), "/")
	if !ok || !strings.HasPrefix(mxc, "mxc://") || id == "" {
		return "", fmt.Errorf("%q is not an mxc:// URI", mxc)
	}

	return "/media/" + url.PathEscape(server) + "/" + url.PathEscape(id), nil
}
