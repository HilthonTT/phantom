package live

import (
	"errors"
	"testing"
	"time"

	"github.com/HilthonTT/phantom/cli/internal/client"
	"github.com/HilthonTT/phantom/cli/internal/tui/resource"
)

func state(t *testing.T) State {
	t.Helper()

	c, err := client.New("localhost:8008")
	if err != nil {
		t.Fatal(err)
	}

	return New(c)
}

var up = client.Status{
	Software: "phantom", Version: "0.1.0",
	ServerName: "phantom.test", Federation: true,
	Spec: "v1.18", Unstable: 2, LocalUsers: 7,
	Latency: 3 * time.Millisecond,
}

func cell(t *testing.T, l resource.Listing, key string) resource.Row {
	t.Helper()

	for _, r := range l.Rows {
		if r.Cells[0] == key {
			return r
		}
	}
	t.Fatalf("no %q row", key)

	return resource.Row{}
}

func TestOverviewShowsWhatTheServerReported(t *testing.T) {
	s := state(t).Apply(ProbedMsg{Status: up})
	l := s.Listing(resource.Overview)

	for key, want := range map[string]string{
		"Server name":   "phantom.test",
		"Version":       "phantom 0.1.0",
		"Client API":    "v1.18, 2 unstable features",
		"Local users":   "7",
		"HTTP listener": "serving at localhost:8008, 3 ms",
	} {
		if got := cell(t, l, key).Cells[1]; got != want {
			t.Errorf("%s = %q, want %q", key, got, want)
		}
	}

	uptime := cell(t, l, "Uptime")
	if src := uptime.Detail[len(uptime.Detail)-1]; src.Value != "sample data" {
		t.Errorf("Uptime source = %q, want sample data", src.Value)
	}
}

func TestOverviewWhenUnreachable(t *testing.T) {
	s := state(t).Apply(ProbedMsg{Err: errors.New("connection refused")})
	l := s.Listing(resource.Overview)

	listener := cell(t, l, "HTTP listener")
	if listener.State != resource.Failed {
		t.Errorf("listener state = %d, want Failed", listener.State)
	}
	if got := cell(t, l, "Server name").Cells[1]; got != "unknown, server unreachable" {
		t.Errorf("Server name = %q", got)
	}
}

func TestAPIServedFollowsTheLink(t *testing.T) {
	served := func(s State, module string) string {
		for _, r := range s.Listing(resource.API).Rows {
			if r.Cells[0] != module {
				continue
			}
			for _, f := range r.Detail {
				if f.Label == "Served" {
					return f.Value
				}
			}
		}
		t.Fatalf("no Served field for %s", module)

		return ""
	}

	s := state(t)
	if got := served(s, "client::sync"); got != "unknown, still connecting" {
		t.Errorf("connecting: %q", got)
	}

	s = s.Apply(ProbedMsg{Status: up})
	if got := served(s, "client::sync"); got != "yes, at localhost:8008" {
		t.Errorf("connected: %q", got)
	}

	off := up
	off.Federation = false
	s = s.Apply(ProbedMsg{Status: off})
	if got := served(s, "federation::server"); got != "no, federation is off" {
		t.Errorf("federation off: %q", got)
	}
}

func TestLatencyAloneIsNoChange(t *testing.T) {
	a := state(t).Apply(ProbedMsg{Status: up})

	slower := up
	slower.Latency *= 10
	if a.Apply(ProbedMsg{Status: slower}).Differs(a) {
		t.Error("a latency change reloads the listings")
	}

	if !a.Apply(ProbedMsg{Err: errors.New("down")}).Differs(a) {
		t.Error("going down is not a change")
	}
}
