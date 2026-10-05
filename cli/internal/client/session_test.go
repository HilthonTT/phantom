package client

import (
	"context"
	"encoding/json"
	"errors"
	"net/http"
	"net/http/httptest"
	"testing"
)

// accounts serves login, logout and whoami for two accounts: "admin", whose
// token the admin API accepts, and "user", whose token it refuses.
func accounts(t *testing.T) *Client {
	t.Helper()

	mux := http.NewServeMux()
	mux.HandleFunc("POST /_matrix/client/v3/login", func(w http.ResponseWriter, r *http.Request) {
		var body struct {
			Identifier struct{ User string } `json:"identifier"`
			Password   string                `json:"password"`
			Device     string                `json:"initial_device_display_name"`
		}
		_ = json.NewDecoder(r.Body).Decode(&body)

		if body.Password != "secret" || body.Device != DeviceName {
			w.WriteHeader(http.StatusForbidden)
			_, _ = w.Write([]byte(`{"errcode":"M_FORBIDDEN","error":"Wrong username or password."}`))
			return
		}

		_ = json.NewEncoder(w).Encode(Session{
			UserID: "@" + body.Identifier.User + ":test", DeviceID: "DEV", AccessToken: body.Identifier.User + "-token",
		})
	})
	mux.HandleFunc("GET /_phantom/admin/v1/whoami", func(w http.ResponseWriter, r *http.Request) {
		switch r.Header.Get("Authorization") {
		case "Bearer admin-token":
			_, _ = w.Write([]byte(`{"user_id":"@admin:test","device_id":"DEV","server_name":"test"}`))
		case "Bearer user-token":
			w.WriteHeader(http.StatusForbidden)
			_, _ = w.Write([]byte(`{"errcode":"M_FORBIDDEN","error":"Only server admins can use the admin API."}`))
		default:
			w.WriteHeader(http.StatusUnauthorized)
			_, _ = w.Write([]byte(`{"errcode":"M_UNKNOWN_TOKEN","error":"Unknown access token."}`))
		}
	})
	mux.HandleFunc("POST /_matrix/client/v3/logout", func(w http.ResponseWriter, _ *http.Request) {
		_, _ = w.Write([]byte(`{}`))
	})

	srv := httptest.NewServer(mux)
	t.Cleanup(srv.Close)

	c, err := New(srv.URL)
	if err != nil {
		t.Fatal(err)
	}

	return c
}

func TestLoginThenWhoAmI(t *testing.T) {
	c := accounts(t)
	ctx := context.Background()

	s, err := c.Login(ctx, "admin", "secret")
	if err != nil {
		t.Fatal(err)
	}
	if s.AccessToken != "admin-token" {
		t.Fatalf("session = %+v", s)
	}

	a, err := c.WhoAmI(ctx)
	if err != nil || a.UserID != "@admin:test" {
		t.Fatalf("WhoAmI = %+v, %v", a, err)
	}
}

func TestWrongPasswordShowsTheServersReason(t *testing.T) {
	_, err := accounts(t).Login(context.Background(), "admin", "nope")
	if err == nil || err.Error() != "Wrong username or password." {
		t.Fatalf("Login err = %v, want the server's message", err)
	}
}

func TestNonAdminIsErrNotAdmin(t *testing.T) {
	c := accounts(t)
	ctx := context.Background()

	if _, err := c.Login(ctx, "user", "secret"); err != nil {
		t.Fatal(err)
	}
	if _, err := c.WhoAmI(ctx); !errors.Is(err, ErrNotAdmin) {
		t.Fatalf("WhoAmI err = %v, want ErrNotAdmin", err)
	}
}

func TestStaleTokenIsUnknown(t *testing.T) {
	c := accounts(t)
	c.Resume(Session{AccessToken: "gone"})

	if _, err := c.WhoAmI(context.Background()); !IsUnknownToken(err) {
		t.Fatalf("WhoAmI err = %v, want an unknown token", err)
	}
}

func TestLogoutDropsTheSession(t *testing.T) {
	c := accounts(t)
	ctx := context.Background()

	if _, err := c.Login(ctx, "admin", "secret"); err != nil {
		t.Fatal(err)
	}
	if err := c.Logout(ctx); err != nil {
		t.Fatal(err)
	}
	if _, ok := c.Session(); ok {
		t.Error("session survived Logout")
	}
}
