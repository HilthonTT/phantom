package app

import (
	"errors"
	"path/filepath"
	"testing"

	tea "charm.land/bubbletea/v2"

	"github.com/HilthonTT/phantom/cli/internal/client"
	"github.com/HilthonTT/phantom/cli/internal/session"
	"github.com/HilthonTT/phantom/cli/internal/tui/live"
	"github.com/HilthonTT/phantom/cli/internal/tui/modal"
	"github.com/HilthonTT/phantom/cli/internal/tui/resource"
)

// offline is a client no test ever reaches: Update is driven by hand and Init
// is never run, so nothing dials it.
func offline(t *testing.T) *client.Client {
	t.Helper()

	c, err := client.New("localhost:8008")
	if err != nil {
		t.Fatal(err)
	}

	return c
}

// noSessions is an empty sessions file of the test's own.
func noSessions(t *testing.T) session.Store {
	t.Helper()

	return session.At(filepath.Join(t.TempDir(), "sessions.json"))
}

func feed(t *testing.T, m Model, msg tea.Msg) (Model, tea.Cmd) {
	t.Helper()

	next, cmd := m.Update(msg)
	m, ok := next.(Model)
	if !ok {
		t.Fatalf("Update returned %T, want app.Model", next)
	}

	return m, cmd
}

func TestProbeResultReachesTheOverview(t *testing.T) {
	m := sized(t, 140, 40)

	m, _ = feed(t, m, live.ProbedMsg{Status: client.Status{
		Software: "phantom", Version: "0.1.0", ServerName: "phantom.test", LocalUsers: -1,
	}})

	if m.connection.Render() == "" {
		t.Fatal("connection panel rendered nothing")
	}

	row, ok := m.workspace.Selected()
	if !ok || m.workspace.Section() != resource.Overview {
		t.Fatal("the overview is not open")
	}
	if row.Cells[0] != "Server name" || row.Cells[1] != "phantom.test" {
		t.Errorf("first overview row = %q, want the reported server name", row.Cells)
	}
}

func TestOnlyScheduledProbesKeepPolling(t *testing.T) {
	m := sized(t, 140, 40)

	if _, cmd := feed(t, m, live.ProbedMsg{Err: errors.New("refused"), Scheduled: true}); cmd == nil {
		t.Error("a scheduled probe did not schedule the next one")
	}
	if _, cmd := feed(t, m, live.ProbedMsg{Err: errors.New("refused")}); cmd != nil {
		t.Error("a manual refresh started a second polling loop")
	}
}

func connected(t *testing.T, m Model) Model {
	t.Helper()

	m, _ = feed(t, m, live.ProbedMsg{Status: client.Status{Software: "phantom", LocalUsers: -1}})

	return m
}

func TestAReachableServerAsksForALoginOnce(t *testing.T) {
	m := connected(t, sized(t, 140, 40))
	if m.modal != modal.Login {
		t.Fatalf("modal = %d after connecting with no session, want the login form", m.modal)
	}

	m = press(t, m, "esc")
	m = connected(t, m)
	if m.modal != modal.None {
		t.Error("a dismissed login form opened again on the next probe")
	}
}

func TestLoginSavesTheSessionAndClosesTheForm(t *testing.T) {
	m := connected(t, sized(t, 140, 40))
	s := client.Session{UserID: "@admin:test", DeviceID: "D", AccessToken: "tok"}

	m, _ = feed(t, m, live.AuthMsg{Session: s, Admin: true})

	if m.modal != modal.None {
		t.Errorf("modal = %d after logging in, want none", m.modal)
	}
	if !m.live.Account.Admin || m.live.Account.User != "@admin:test" {
		t.Errorf("account = %+v", m.live.Account)
	}
	if saved, ok, _ := m.store.Load(m.client.URL()); !ok || saved != s {
		t.Errorf("saved session = %+v, %v", saved, ok)
	}
}

func TestARefusedLoginKeepsTheFormOpen(t *testing.T) {
	m := connected(t, sized(t, 140, 40))

	m, _ = feed(t, m, live.AuthMsg{Refused: true, Err: errors.New("Wrong username or password.")})

	if m.modal != modal.Login {
		t.Fatalf("modal = %d after a refused login, want the form", m.modal)
	}
	if m.live.Account.SignedIn() {
		t.Error("signed in after a refused login")
	}
}

func TestAnEndedSavedSessionIsForgotten(t *testing.T) {
	m := sized(t, 140, 40)
	_ = m.store.Save(m.client.URL(), client.Session{AccessToken: "old"})

	m, _ = feed(t, m, live.AuthMsg{Resumed: true, Refused: true, Err: errors.New("Unknown access token.")})

	if _, ok, _ := m.store.Load(m.client.URL()); ok {
		t.Error("the ended session is still saved")
	}
	if m.modal != modal.Login {
		t.Errorf("modal = %d, want the login form", m.modal)
	}
}
