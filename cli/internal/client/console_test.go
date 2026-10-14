package client

import (
	"context"
	"net/http"
	"testing"
)

func TestTheConsoleRoutesAskWhatTheyShould(t *testing.T) {
	var got []string
	mux := http.NewServeMux()
	mux.HandleFunc("/_phantom/admin/v1/", func(w http.ResponseWriter, r *http.Request) {
		got = append(got, r.Method+" "+r.URL.RequestURI())

		switch r.URL.Path {
		case "/_phantom/admin/v1/audit":
			_, _ = w.Write([]byte(`{"records":[{"id":"1","ok":true}],"next":"1"}`))
		case "/_phantom/admin/v1/rooms/!r:test/events":
			_, _ = w.Write([]byte(`{"events":[{"event_id":"$e","content":{"body":"hi"},"prev_events":["$p"]}],"next":null}`))
		case "/_phantom/admin/v1/logs/capture":
			_, _ = w.Write([]byte(`{"level":"debug"}`))
		default:
			_, _ = w.Write([]byte(`[]`))
		}
	})
	c := serve(t, mux)
	ctx := context.Background()

	_, _ = c.LogsSince(ctx, 41, 500)
	page, _ := c.Audit(ctx, "0007", 50)
	events, _ := c.RoomEvents(ctx, "!r:test", "", 20)
	level, _ := c.LogCapture(ctx)
	_ = c.SetLogCapture(ctx, "info")

	want := []string{
		"GET /_phantom/admin/v1/logs?limit=500&since=41",
		"GET /_phantom/admin/v1/audit?before=0007&limit=50",
		"GET /_phantom/admin/v1/rooms/%21r:test/events?limit=20",
		"GET /_phantom/admin/v1/logs/capture",
		"PUT /_phantom/admin/v1/logs/capture",
	}
	if len(got) != len(want) {
		t.Fatalf("calls = %v", got)
	}
	for i := range want {
		if got[i] != want[i] {
			t.Errorf("call %d = %q, want %q", i, got[i], want[i])
		}
	}

	if len(page.Records) != 1 || page.Next == nil || *page.Next != "1" {
		t.Errorf("audit page = %+v", page)
	}
	if len(events.Events) != 1 || string(events.Events[0].Content) != `{"body":"hi"}` || events.Next != nil {
		t.Errorf("events = %+v", events)
	}
	if level != "debug" {
		t.Errorf("level = %q", level)
	}
}
