package app

import (
	"errors"
	"testing"

	tea "charm.land/bubbletea/v2"

	"github.com/HilthonTT/phantom/cli/internal/client"
	"github.com/HilthonTT/phantom/cli/internal/tui/live"
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
