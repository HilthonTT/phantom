package listings

import (
	"testing"
	"time"

	"github.com/HilthonTT/phantom/cli/internal/client"
	"github.com/HilthonTT/phantom/cli/internal/tui/resource"
)

func TestRoutesAreByAudienceThenPath(t *testing.T) {
	sync := "sync_events::v3"
	l := Routes([]client.Route{
		{Method: "GET", Path: "/_matrix/client/v3/sync", Group: "client", Endpoint: &sync, Stable: true},
		{Method: "GET", Path: "/_phantom/admin/v1/routes", Group: "admin", Stable: true},
		{Method: "*", Path: "/_matrix/federation/{*path}", Group: "federation", Stable: true},
		{Method: "POST", Path: "/_matrix/client/unstable/org.matrix.msc4186/sync", Group: "client"},
	})

	order := []string{l.Rows[0].Cells[1], l.Rows[1].Cells[1], l.Rows[2].Cells[1], l.Rows[3].Cells[1]}
	want := []string{
		"/_phantom/admin/v1/routes",
		"/_matrix/client/unstable/org.matrix.msc4186/sync",
		"/_matrix/client/v3/sync",
		"/_matrix/federation/{*path}",
	}
	for i := range want {
		if order[i] != want[i] {
			t.Fatalf("order = %v, want %v", order, want)
		}
	}
	if l.Rows[1].State != resource.Held || l.Rows[3].Cells[0] != "any" {
		t.Errorf("unstable state %d, any method %q", l.Rows[1].State, l.Rows[3].Cells[0])
	}

	if got := RouteCounts([]client.Route{{Group: "client"}, {Group: "client"}, {Group: "admin"}}); got != "3 routes: 2 client, 1 admin" {
		t.Errorf("counts = %q", got)
	}
}

func TestTheAuditLogShowsWhatFailed(t *testing.T) {
	l := Audit([]client.AuditRecord{
		{ID: "2", AtMs: now.UnixMilli(), Admin: "@alice:test", Via: "api", Action: "DELETE /rooms/{room_id}", Target: "!r:test", OK: false, Result: "403 Forbidden: No."},
		{ID: "1", AtMs: now.Add(-time.Hour).UnixMilli(), Admin: "@alice:test", Via: "command", Action: "!admin server backup", OK: true},
	}, now)

	if l.Rows[0].State != resource.Failed || l.Rows[0].Cells[4] != "failed" || l.Rows[0].Cells[1] != "@alice" {
		t.Errorf("failed row = %+v", l.Rows[0])
	}
	if l.Rows[1].Cells[3] != "—" || l.Rows[1].Ref[0] != "1" {
		t.Errorf("server-wide row = %+v", l.Rows[1])
	}
}

func TestFederationHealthReadsTheFailures(t *testing.T) {
	recent := at(time.Minute)
	reason := "connection refused"

	for _, tc := range []struct {
		peer  client.Peer
		word  string
		state resource.State
	}{
		{client.Peer{LastContactMs: recent}, "reachable", resource.Done},
		{client.Peer{LastContactMs: recent, ConsecutiveFailures: 2, LastError: &reason}, "flaky", resource.Held},
		{client.Peer{ConsecutiveFailures: 7}, "failing", resource.Failed},
		{client.Peer{LastInboundMs: recent}, "inbound only", resource.Done},
		{client.Peer{}, "not contacted", resource.NoState},
	} {
		if word, state := PeerHealth(tc.peer, now); word != tc.word || state != tc.state {
			t.Errorf("%+v reads %q %d, want %q %d", tc.peer, word, state, tc.word, tc.state)
		}
	}

	l := Federation([]client.Peer{
		{Server: "fine.example", LastContactMs: recent},
		{Server: "flaky.example", LastContactMs: recent, ConsecutiveFailures: 1, Queued: 12},
	}, now)
	if l.Rows[0].Cells[0] != "flaky.example" || l.Rows[0].Cells[2] != "12" {
		t.Errorf("a failing server is not first with its queue: %v", l.Rows[0].Cells)
	}
}
