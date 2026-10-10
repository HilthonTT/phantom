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

func TestTheAdminStatsFillTheRestOfTheOverview(t *testing.T) {
	s := state(t).Apply(ProbedMsg{Status: up})
	s, gen := s.StartAdmin()
	backupAt := time.Now().Add(-time.Hour).UnixMilli()

	s, _ = s.TakeAdmin(AdminMsg{Gen: gen, Section: resource.Overview, Stats: &client.Stats{
		KeyBackupUsers: 2, MediaFiles: 3, MediaBytes: 2048, OpenReports: 1,
		LastBackupMs: &backupAt, Login: []string{"password", "OIDC"},
	}})
	s, _ = s.TakeAdmin(AdminMsg{Gen: gen, Section: resource.Services, Services: &ServiceCounts{Total: 57, Running: 14}})

	l := s.Listing(resource.Overview)
	for key, want := range map[string]string{
		"Key backups":  "2 users",
		"Media store":  "2.0 KiB in 3 files",
		"Open reports": "1",
		"Login":        "password, OIDC",
		"Services":     "57, 14 with workers running, 0 failed",
	} {
		if got := cell(t, l, key).Cells[1]; got != want {
			t.Errorf("%s = %q, want %q", key, got, want)
		}
	}

	for _, r := range l.Rows {
		if r.Cells[0] == "Events today" {
			t.Error("an overview row the server cannot answer is still shown")
		}
	}
}

func TestTheOverviewCountsTheServersFederatedWith(t *testing.T) {
	s := state(t).Apply(ProbedMsg{Status: up})
	s, gen := s.StartAdmin()
	s, _ = s.TakeAdmin(AdminMsg{Gen: gen, Section: resource.Overview, Stats: &client.Stats{Federation: true, MediaFiles: 1}})

	if got := cell(t, s.Listing(resource.Overview), "Media store").Cells[1]; got != "0 B in 1 file" {
		t.Errorf("media store = %q", got)
	}

	s, _ = s.TakeAdmin(AdminMsg{Gen: gen, Section: resource.Federation, Peers: &PeerCounts{Known: 3, Reachable: 2, Failing: 1}})
	if got := cell(t, s.Listing(resource.Overview), "Federation").Cells[1]; got != "3 servers, 2 reachable, 1 backing off" {
		t.Errorf("federation = %q", got)
	}
}
