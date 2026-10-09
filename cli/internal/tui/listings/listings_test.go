package listings

import (
	"testing"
	"time"

	"github.com/HilthonTT/phantom/cli/internal/client"
	"github.com/HilthonTT/phantom/cli/internal/tui/resource"
)

var now = time.Date(2026, 10, 9, 12, 0, 0, 0, time.UTC)

func at(d time.Duration) *int64 {
	ms := now.Add(-d).UnixMilli()
	return &ms
}

func str(s string) *string { return &s }

func TestUsersAreNewestFirstWithTheirState(t *testing.T) {
	l := Users([]client.AdminUser{
		{UserID: "@old:test", LastSeenMs: at(72 * time.Hour)},
		{UserID: "@gone:test", Deactivated: true},
		{UserID: "@new:test", Admin: true, LastSeenMs: at(2 * time.Minute)},
		{UserID: "@phantom:test", ServerUser: true},
	}, now)

	want := [][]string{
		{"@new:test", "yes", "active", "2 min ago"},
		{"@old:test", "no", "active", "3 days ago"},
		{"@gone:test", "no", "deactivated", "never"},
		{"@phantom:test", "no", "server user", "never"},
	}
	for i, w := range want {
		for j, cell := range w {
			if got := l.Rows[i].Cells[j]; got != cell {
				t.Errorf("row %d cell %d = %q, want %q", i, j, got, cell)
			}
		}
	}
	if l.Rows[2].State != resource.Failed {
		t.Errorf("a deactivated user's state = %d, want Failed", l.Rows[2].State)
	}
}

func TestTokensFlagWhatIsAboutToRunOut(t *testing.T) {
	one, nine, ten := int64(1), int64(9), int64(10)
	soon := now.Add(3 * time.Hour).UnixMilli()

	l := Tokens([]client.RegistrationToken{
		{Token: "forever", Source: "database", Uses: &one},
		{Token: "last-use", Source: "database", Uses: &nine, MaxUses: &ten},
		{Token: "today", Source: "database", Uses: &one, ExpiresAtMs: &soon},
		{Token: "***********", Source: "config"},
	}, now)

	if l.Rows[0].Cells[0] != "today" {
		t.Errorf("first token = %q, want the one expiring soonest", l.Rows[0].Cells[0])
	}

	states := map[string]resource.State{}
	for _, r := range l.Rows {
		states[r.Cells[0]] = r.State
	}
	if states["today"] != resource.Held || states["last-use"] != resource.Held || states["forever"] != resource.Done {
		t.Errorf("states = %v", states)
	}
}

func TestRoomsAreNamedByWhatTheyHave(t *testing.T) {
	l := Rooms([]client.AdminRoom{
		{RoomID: "!a:test", Name: str("General"), JoinedMembers: 3, JoinRule: "public", Published: true},
		{RoomID: "!b:test", CanonicalAlias: str("#ops:test"), JoinedMembers: 9, JoinRule: "invite", Banned: true},
		{RoomID: "!c:test", JoinedMembers: 1, JoinRule: "invite"},
	})

	titles := []string{l.Rows[0].Cells[0], l.Rows[1].Cells[0], l.Rows[2].Cells[0]}
	if titles[0] != "#ops:test" || titles[1] != "General" || titles[2] != "!c:test" {
		t.Errorf("titles = %v, want by members, named by name, alias, then ID", titles)
	}
	if l.Rows[0].State != resource.Failed {
		t.Error("a banned room is not flagged")
	}
	if l.Rows[1].Cells[3] != "public, listed" {
		t.Errorf("visibility = %q", l.Rows[1].Cells[3])
	}
}

func TestUptimeAndBytes(t *testing.T) {
	for secs, want := range map[int64]string{59: "00:00", 3660: "01:01", 6*86400 + 4*3600 + 11*60: "6d 04:11"} {
		if got := Uptime(secs); got != want {
			t.Errorf("Uptime(%d) = %q, want %q", secs, got, want)
		}
	}
	for n, want := range map[int64]string{512: "512 B", 2048: "2.0 KiB", 1932735283: "1.8 GiB"} {
		if got := Bytes(n); got != want {
			t.Errorf("Bytes(%d) = %q, want %q", n, got, want)
		}
	}
}

func TestSettingsDropTheQuotesOfPlainStrings(t *testing.T) {
	l := Settings([]client.Setting{
		{Key: "server_name", Value: `"phantom.test"`},
		{Key: "port", Value: "ListeningPort { ports: Left(8008) }"},
		{Key: "names", Value: `["a", "b"]`},
	})

	got := map[string]string{}
	for _, r := range l.Rows {
		got[r.Cells[0]] = r.Cells[1]
	}
	if got["server_name"] != "phantom.test" || got["names"] != `["a", "b"]` {
		t.Errorf("values = %v", got)
	}
}
