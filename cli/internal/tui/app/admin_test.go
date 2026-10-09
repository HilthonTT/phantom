package app

import (
	"testing"

	"github.com/HilthonTT/phantom/cli/internal/client"
	"github.com/HilthonTT/phantom/cli/internal/tui/live"
	"github.com/HilthonTT/phantom/cli/internal/tui/resource"
)

func signedInAs(t *testing.T, admin bool) Model {
	t.Helper()

	m := connected(t, sized(t, 140, 40))
	m, _ = feed(t, m, live.AuthMsg{Session: client.Session{UserID: "@alice:test"}, Admin: admin})
	m.workspace.Open(resource.Users)

	return m
}

func TestAnAdminSeesTheServersUsers(t *testing.T) {
	m := signedInAs(t, true)

	if note := m.live.Listing(resource.Users).Note; note != "loading…" {
		t.Errorf("note before the answer = %q, want loading", note)
	}

	listing := resource.Listing{Rows: []resource.Row{{Cells: []string{"@alice:test"}}}}
	m, _ = feed(t, m, live.AdminMsg{Gen: m.live.Admin.Gen, Section: resource.Users, Listing: listing})

	row, ok := m.workspace.Selected()
	if !ok || row.Cells[0] != "@alice:test" {
		t.Errorf("users tab shows %v, want the server's users", row.Cells)
	}
}

func TestAnAnswerForAnEarlierSignInIsDropped(t *testing.T) {
	m := signedInAs(t, true)
	stale := m.live.Admin.Gen - 1

	listing := resource.Listing{Rows: []resource.Row{{Cells: []string{"@someone-else:test"}}}}
	m, _ = feed(t, m, live.AdminMsg{Gen: stale, Section: resource.Users, Listing: listing})

	if row, _ := m.workspace.Selected(); row.Cells[0] == "@someone-else:test" {
		t.Error("a stale answer reached the users tab")
	}
}

func TestANonAdminIsToldTheRowsAreSamples(t *testing.T) {
	m := signedInAs(t, false)

	if note := m.live.Listing(resource.Users).Note; note != "sample · sign in as an admin" {
		t.Errorf("note = %q", note)
	}
	if cmd := m.refreshAdmin(); cmd != nil {
		t.Error("a non-admin's refresh asked the admin API")
	}
}

func TestSigningOutForgetsTheAdminAnswers(t *testing.T) {
	m := signedInAs(t, true)
	listing := resource.Listing{Rows: []resource.Row{{Cells: []string{"@alice:test"}}}}
	m, _ = feed(t, m, live.AdminMsg{Gen: m.live.Admin.Gen, Section: resource.Users, Listing: listing})

	m, _ = feed(t, m, live.LoggedOutMsg{})

	if row, _ := m.workspace.Selected(); row.Cells[0] == "@alice:test" {
		t.Error("the server's users are still shown after signing out")
	}
}
