package listings

import (
	"strings"
	"testing"
	"time"

	"github.com/HilthonTT/phantom/cli/internal/client"
	"github.com/HilthonTT/phantom/cli/internal/tui/resource"
)

func TestServicesReadAsTheirWorkersRun(t *testing.T) {
	failure := "boom"
	l := Services([]client.AdminService{
		{Name: "media", Status: "running"},
		{Name: "rooms::alias", Status: "finished"},
		{Name: "net::sending", Status: "failed", Error: &failure, Restarts: 2},
	}, now)

	want := []struct {
		word  string
		state resource.State
	}{{"running", resource.Running}, {"ready", resource.Done}, {"failed", resource.Failed}}
	for i, w := range want {
		if l.Rows[i].Cells[1] != w.word || l.Rows[i].State != w.state {
			t.Errorf("row %d = %v state %d, want %s", i, l.Rows[i].Cells, l.Rows[i].State, w.word)
		}
	}
	if l.Rows[0].Cells[2] == "—" {
		t.Error("a registered service has no purpose")
	}

	total, running, failed := ServiceCounts([]client.AdminService{{Status: "running"}, {Status: "failed"}, {Status: "finished"}})
	if total != 3 || running != 1 || failed != 1 {
		t.Errorf("counts = %d %d %d", total, running, failed)
	}
}

func TestFederationPutsFailingServersFirst(t *testing.T) {
	recent, old := at(time.Minute), at(time.Hour)
	l := Federation([]client.Peer{
		{Server: "old.example", LastContactMs: old},
		{Server: "new.example", LastContactMs: recent},
		{Server: "down.example", Backoff: &struct {
			Permanent bool  `json:"permanent"`
			SinceMs   int64 `json:"since_ms"`
			DelaySecs int64 `json:"delay_secs"`
		}{SinceMs: now.Add(-time.Minute).UnixMilli(), DelaySecs: 600}},
		{Server: "quiet.example"},
	}, now)

	order := []string{l.Rows[0].Cells[0], l.Rows[1].Cells[0], l.Rows[2].Cells[0], l.Rows[3].Cells[0]}
	want := []string{"down.example", "new.example", "old.example", "quiet.example"}
	for i := range want {
		if order[i] != want[i] {
			t.Fatalf("order = %v, want %v", order, want)
		}
	}
	if l.Rows[0].Cells[1] != "backing off" || l.Rows[3].Cells[1] != "not contacted" {
		t.Errorf("statuses = %q, %q", l.Rows[0].Cells[1], l.Rows[3].Cells[1])
	}
}

func TestFederationBackoffCountsFromTheLastFailure(t *testing.T) {
	backoff := func(ago time.Duration, delay int64) client.Peer {
		p := client.Peer{Server: "down.example"}
		p.Backoff = &struct {
			Permanent bool  `json:"permanent"`
			SinceMs   int64 `json:"since_ms"`
			DelaySecs int64 `json:"delay_secs"`
		}{SinceMs: now.Add(-ago).UnixMilli(), DelaySecs: delay}
		return p
	}

	// Failed 50 minutes ago with an hour's backoff: ten minutes to go.
	l := Federation([]client.Peer{backoff(50*time.Minute, 3600)}, now)
	if got := l.Rows[0].Detail[4].Value; !strings.HasPrefix(got, "retry in 10m0s;") {
		t.Errorf("backoff = %q, want retry in 10m0s", got)
	}

	// Failed two hours ago with an hour's backoff: the server tries it again.
	expired := backoff(2*time.Hour, 3600)
	l = Federation([]client.Peer{expired}, now)
	if got := l.Rows[0].Cells[1]; got != "retrying" {
		t.Errorf("status = %q, want retrying", got)
	}

	known, reachable, failing := PeerCounts([]client.Peer{backoff(time.Minute, 600), expired}, now)
	if known != 2 || reachable != 0 || failing != 1 {
		t.Errorf("counts = %d %d %d, want 2 0 1", known, reachable, failing)
	}
}

func TestMediaIsLargestFirstAndNamedByID(t *testing.T) {
	alice := "@alice:test"
	l := Media([]client.StoredMedia{
		{MXC: "mxc://test/small", Size: 10, Local: true, Uploader: &alice},
		{MXC: "mxc://remote.example/big", Size: 4096},
	}, now)

	if l.Rows[0].Cells[0] != "big" || l.Rows[0].Cells[1] != "4.0 KiB" || l.Rows[1].Cells[3] != "@alice" {
		t.Errorf("rows = %v, %v", l.Rows[0].Cells, l.Rows[1].Cells)
	}
	if l.Rows[0].Ref[0] != "mxc://remote.example/big" {
		t.Errorf("ref = %v", l.Rows[0].Ref)
	}
}

func TestLogsColourTheirLevels(t *testing.T) {
	l := Logs([]client.LogLine{
		{AtMs: now.UnixMilli(), Level: "ERROR", Target: "phantom_service::media", Message: "disk full"},
		{AtMs: now.UnixMilli(), Level: "INFO", Target: "phantom_api", Message: "hi"},
	}, now)

	if l.Rows[0].State != resource.Failed || l.Rows[0].Cells[2] != "service::media" || l.Rows[1].State != resource.NoState {
		t.Errorf("rows = %+v", l.Rows)
	}
}

func TestReportsNameWhatWasReported(t *testing.T) {
	event, room, user := "$e", "!r:test", "@bob:test"
	l := Reports([]client.Report{
		{ID: "1", Kind: "event", EventID: &event, RoomID: &room, UserID: &user, Reason: "spam"},
		{ID: "2", Kind: "room", RoomID: &room},
		{ID: "3", Kind: "user", UserID: &user},
	}, now)

	targets := []string{l.Rows[0].Cells[2], l.Rows[1].Cells[2], l.Rows[2].Cells[2]}
	if targets[0] != "$e" || targets[1] != "!r:test" || targets[2] != "@bob:test" {
		t.Errorf("targets = %v", targets)
	}
	if l.Rows[1].Ref[0] != "2" {
		t.Errorf("ref = %v", l.Rows[1].Ref)
	}
}
