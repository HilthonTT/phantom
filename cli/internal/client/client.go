// Package client talks to a running phantom-server over its HTTP API.
//
// Only the endpoints that answer without an access token are probed: the
// server's own version route, the client versions, the federation signing
// keys and the local user count. Everything else the TUI shows is still
// sample data until it can log in.
package client

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"net/http"
	"net/url"
	"strings"
	"sync"
	"time"
)

// DefaultURL is where phantom-server listens when its config sets neither
// address nor port.
const DefaultURL = "http://localhost:8008"

const requestTimeout = 3 * time.Second

type Client struct {
	base *url.URL
	http *http.Client

	// mu guards session: probes run on their own goroutines while a login or
	// logout replaces it.
	mu      sync.RWMutex
	session *Session
}

// New parses raw as the server's base URL, taking a bare host[:port] as
// plain http, since phantom-server serves plain TCP.
func New(raw string) (*Client, error) {
	raw = strings.TrimSpace(raw)
	if raw == "" {
		raw = DefaultURL
	}
	if !strings.Contains(raw, "://") {
		raw = "http://" + raw
	}

	base, err := url.Parse(raw)
	if err != nil {
		return nil, fmt.Errorf("server URL %q: %w", raw, err)
	}
	if base.Scheme != "http" && base.Scheme != "https" {
		return nil, fmt.Errorf("server URL %q: scheme must be http or https", raw)
	}
	if base.Host == "" {
		return nil, fmt.Errorf("server URL %q: no host", raw)
	}

	base.Path = strings.TrimSuffix(base.Path, "/")

	// Requests are bounded by their contexts instead of the client, since a
	// sync long-poll waits far longer than any other request.
	return &Client{base: base, http: &http.Client{}}, nil
}

func (c *Client) URL() string { return c.base.String() }

// Host is the URL without its scheme, for where space is short.
func (c *Client) Host() string { return c.base.Host + c.base.Path }

// Status is what a probe learned about the server.
type Status struct {
	// Software and Version come from /_phantom/server_version.
	Software string
	Version  string

	// ServerName is empty when federation is off, since the signing keys
	// that carry it are withheld then.
	ServerName string
	Federation bool

	// Spec is the newest Matrix version the client API offers, and Unstable
	// the number of unstable features it advertises.
	Spec     string
	Unstable int

	// LocalUsers is -1 when the server withholds the count, as it does with
	// federation off.
	LocalUsers int

	Latency time.Duration
}

// Probe asks the server who it is. Only the version route is required to
// answer; the others fill in what they can.
func (c *Client) Probe(ctx context.Context) (Status, error) {
	s := Status{LocalUsers: -1}

	start := time.Now()

	var version struct {
		Name    string `json:"name"`
		Version string `json:"version"`
	}
	if err := c.get(ctx, "/_phantom/server_version", &version); err != nil {
		return Status{}, err
	}

	s.Latency = time.Since(start)
	s.Software, s.Version = version.Name, version.Version

	var versions struct {
		Versions []string        `json:"versions"`
		Unstable map[string]bool `json:"unstable_features"`
	}
	if c.get(ctx, "/_matrix/client/versions", &versions) == nil {
		if n := len(versions.Versions); n > 0 {
			s.Spec = versions.Versions[n-1]
		}
		for _, on := range versions.Unstable {
			if on {
				s.Unstable++
			}
		}
	}

	var keys struct {
		ServerName string `json:"server_name"`
	}
	if c.get(ctx, "/_matrix/key/v2/server", &keys) == nil {
		s.ServerName, s.Federation = keys.ServerName, true
	}

	var users struct {
		Count int `json:"count"`
	}
	if c.get(ctx, "/_phantom/local_user_count", &users) == nil {
		s.LocalUsers = users.Count
	}

	return s, nil
}

// StatusError is a response other than 200, carrying the Matrix error code
// and message when the body had them.
type StatusError struct {
	Path    string
	Code    int
	ErrCode string
	Message string
}

func (e *StatusError) Error() string {
	switch {
	case e.Message != "":
		return e.Message
	case e.ErrCode != "":
		return fmt.Sprintf("%s: %d %s", e.Path, e.Code, e.ErrCode)
	default:
		return fmt.Sprintf("%s: %d %s", e.Path, e.Code, http.StatusText(e.Code))
	}
}

func statusError(path string, resp *http.Response) error {
	var body struct {
		ErrCode string `json:"errcode"`
		Error   string `json:"error"`
	}
	_ = json.NewDecoder(resp.Body).Decode(&body)

	return &StatusError{Path: path, Code: resp.StatusCode, ErrCode: body.ErrCode, Message: body.Error}
}

func (c *Client) get(ctx context.Context, path string, into any) error {
	return c.do(ctx, http.MethodGet, path, nil, into)
}

// unwrapURLError drops the method and URL net/http wraps every transport
// error in, which the TUI already shows beside it.
func unwrapURLError(err error) error {
	if urlErr, ok := errors.AsType[*url.Error](err); ok {
		return urlErr.Err
	}

	return err
}
