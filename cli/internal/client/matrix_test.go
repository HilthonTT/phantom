package client

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"testing"
	"time"
)

func serve(t *testing.T, mux *http.ServeMux) *Client {
	t.Helper()

	srv := httptest.NewServer(mux)
	t.Cleanup(srv.Close)

	c, err := New(srv.URL)
	if err != nil {
		t.Fatal(err)
	}
	c.Resume(Session{UserID: "@alice:test", AccessToken: "tok"})

	return c
}

func TestSyncAsksForWhatChangedAndDecodesIt(t *testing.T) {
	mux := http.NewServeMux()
	mux.HandleFunc("GET /_matrix/client/v3/sync", func(w http.ResponseWriter, r *http.Request) {
		q := r.URL.Query()
		if q.Get("since") != "s1" || q.Get("timeout") != "30000" || q.Get("filter") == "" {
			t.Errorf("query = %v", q)
		}
		if r.Header.Get("Authorization") != "Bearer tok" {
			t.Errorf("unsigned sync")
		}

		_, _ = w.Write([]byte(`{"next_batch":"s2","rooms":{"join":{"!r:test":{
		  "timeline":{"events":[{"type":"m.room.message","sender":"@bob:test","event_id":"$1",
		    "origin_server_ts":5,"content":{"body":"hi"},"unsigned":{"transaction_id":"t1"}}]},
		  "unread_notifications":{"notification_count":2}}}}}`))
	})

	resp, err := serve(t, mux).Sync(context.Background(), "s1", 30*time.Second)
	if err != nil {
		t.Fatal(err)
	}

	joined, ok := resp.Rooms.Join["!r:test"]
	if resp.NextBatch != "s2" || !ok {
		t.Fatalf("resp = %+v", resp)
	}
	if ev := joined.Timeline.Events[0]; ev.EventID != "$1" || ev.Unsigned.TransactionID != "t1" {
		t.Errorf("event = %+v", ev)
	}
	if joined.UnreadNotifications.NotificationCount != 2 {
		t.Errorf("unread = %d", joined.UnreadNotifications.NotificationCount)
	}
}

func TestTheFirstSyncDoesNotWait(t *testing.T) {
	mux := http.NewServeMux()
	mux.HandleFunc("GET /_matrix/client/v3/sync", func(w http.ResponseWriter, r *http.Request) {
		if q := r.URL.Query(); q.Has("since") || q.Get("timeout") != "0" {
			t.Errorf("query = %v, want no since and no wait", q)
		}
		_, _ = w.Write([]byte(`{"next_batch":"s1"}`))
	})

	if _, err := serve(t, mux).Sync(context.Background(), "", 0); err != nil {
		t.Fatal(err)
	}
}

func TestSendTextPutsTheMessageUnderItsTransaction(t *testing.T) {
	mux := http.NewServeMux()
	mux.HandleFunc("PUT /_matrix/client/v3/rooms/{room}/send/m.room.message/{txn}", func(w http.ResponseWriter, r *http.Request) {
		var body map[string]string
		_ = json.NewDecoder(r.Body).Decode(&body)

		if r.PathValue("room") != "!r:test" || r.PathValue("txn") != "t1" ||
			body["msgtype"] != "m.emote" || body["body"] != "waves" {
			t.Errorf("room %q txn %q body %v", r.PathValue("room"), r.PathValue("txn"), body)
		}
		_, _ = w.Write([]byte(`{"event_id":"$sent"}`))
	})

	id, err := serve(t, mux).SendText(context.Background(), "!r:test", "t1", "m.emote", "waves")
	if err != nil || id != "$sent" {
		t.Fatalf("SendText = %q, %v", id, err)
	}
}

func TestJoinEscapesTheAlias(t *testing.T) {
	mux := http.NewServeMux()
	mux.HandleFunc("POST /_matrix/client/v3/join/{target}", func(w http.ResponseWriter, r *http.Request) {
		if r.PathValue("target") != "#general:test" {
			t.Errorf("target = %q", r.PathValue("target"))
		}
		_, _ = w.Write([]byte(`{"room_id":"!r:test"}`))
	})

	id, err := serve(t, mux).Join(context.Background(), "#general:test")
	if err != nil || id != "!r:test" {
		t.Fatalf("Join = %q, %v", id, err)
	}
}
