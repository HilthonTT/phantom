package app

import (
	"testing"

	tea "charm.land/bubbletea/v2"

	"github.com/HilthonTT/phantom/cli/internal/client"
	"github.com/HilthonTT/phantom/cli/internal/tui/live"
	"github.com/HilthonTT/phantom/cli/internal/tui/modal"
	"github.com/HilthonTT/phantom/cli/internal/tui/resource"
)

// withRooms is an admin looking at a rooms table holding #lobby.
func withRooms(t *testing.T) Model {
	t.Helper()

	m := signedInAs(t, true)
	m.workspace.Open(resource.Rooms)
	listing := resource.Listing{
		Columns: []resource.Column{{Title: "Room"}},
		Rows: []resource.Row{{
			Cells:  []string{"#lobby:test"},
			Ref:    []string{"!r:test", "no"},
			Detail: []resource.Field{{Label: "Name", Value: "Lobby"}, {Label: "Version", Value: "12"}},
		}},
	}
	m, _ = feed(t, m, live.AdminMsg{Gen: m.live.Admin.Gen, Section: resource.Rooms, Listing: listing})
	m.focus = focusWorkspace

	return m
}

func typeLine(t *testing.T, m Model, text string) Model {
	t.Helper()

	for _, r := range text {
		m = press(t, m, string(r))
	}

	return m
}

func TestDeletingARoomAsksForItsName(t *testing.T) {
	m := withRooms(t)
	m, _ = feed(t, m, tea.KeyPressMsg{Code: tea.KeyEnter})
	m = arrow(t, m, tea.KeyDown)
	m = arrow(t, m, tea.KeyDown) // Delete and purge

	m = arrow(t, m, tea.KeyEnter)
	if m.modal != modal.Input || m.inputFor != inputConfirmName || m.acting.Kind != live.DeleteRoom {
		t.Fatalf("modal = %d for %d, want the name asked for", m.modal, m.inputFor)
	}

	wrong := typeLine(t, m, "#other:test")
	wrong, cmd := feed(t, wrong, tea.KeyPressMsg{Code: tea.KeyEnter})
	if cmd != nil || wrong.modal != modal.Notice {
		t.Error("a wrong name deleted the room")
	}

	right := typeLine(t, m, "#lobby:test")
	right, cmd = feed(t, right, tea.KeyPressMsg{Code: tea.KeyEnter})
	if cmd == nil || right.modal != modal.None {
		t.Error("the right name did not delete the room")
	}
}

func TestInspectingARoomFetchesItAndEscapeCloses(t *testing.T) {
	m := withRooms(t)

	m, cmd := feed(t, m, tea.KeyPressMsg{Code: 'i', Text: "i"})
	if !m.roomOpen || cmd == nil || m.room.RoomID != "!r:test" {
		t.Fatalf("open = %v room %q, want the inspector fetching !r:test", m.roomOpen, m.room.RoomID)
	}
	if m.render() == "" {
		t.Error("the inspector rendered nothing")
	}

	m, _ = feed(t, m, live.RoomMsg{RoomID: "!r:test", Servers: []string{"test"}})
	m = arrow(t, m, tea.KeyTab)
	m = arrow(t, m, tea.KeyTab)
	m = arrow(t, m, tea.KeyTab) // Servers
	if row, ok := m.room.Selected(); !ok || row.Cells[0] != "test" {
		t.Errorf("servers view selects %v", row.Cells)
	}

	m = arrow(t, m, tea.KeyEscape)
	if m.roomOpen {
		t.Error("escape left the inspector open")
	}
}

func TestAnAnswerForAnotherRoomIsDropped(t *testing.T) {
	m := withRooms(t)
	m, _ = feed(t, m, tea.KeyPressMsg{Code: 'i', Text: "i"})

	m, _ = feed(t, m, live.RoomMsg{RoomID: "!elsewhere:test", Servers: []string{"x"}})
	if m.room.Extremities() != 0 || !m.room.Loading() {
		t.Error("another room's answer reached the inspector")
	}
}

// withLogs is an admin looking at the server's log.
func withLogs(t *testing.T) Model {
	t.Helper()

	m := signedInAs(t, true)
	m.workspace.Open(resource.Logs)
	m, _ = feed(t, m, live.AdminMsg{Gen: m.live.Admin.Gen, Section: resource.Logs, LogLines: []client.LogLine{
		{Seq: 2, Level: "WARN", Target: "phantom_service::net::sending", Message: "retrying"},
		{Seq: 1, Level: "INFO", Target: "phantom_api", Message: "listening"},
	}})
	m.focus = focusWorkspace

	return m
}

func TestFollowingTheLogTicksUntilPaused(t *testing.T) {
	m := withLogs(t)

	m, cmd := feed(t, m, tea.KeyPressMsg{Code: 'f', Text: "f"})
	if !m.live.Admin.Logs.Follow || cmd == nil {
		t.Fatal("f did not start following")
	}

	tick := live.LogTickMsg{Gen: m.live.Admin.Gen, Tick: m.logTick}
	if _, cmd := feed(t, m, tick); cmd == nil {
		t.Error("a tick while following asked for nothing")
	}

	m = press(t, m, "p")
	if !m.live.Admin.Logs.Paused {
		t.Fatal("p did not pause")
	}
	if _, cmd := feed(t, m, tick); cmd != nil {
		t.Error("a tick while paused asked for lines")
	}

	m, _ = feed(t, m, tea.KeyPressMsg{Code: 'p', Text: "p"})
	if _, cmd := feed(t, m, tick); cmd != nil {
		t.Error("the timer from before the pause is still running")
	}
}

func TestTheLogNarrowsByLevelAndTarget(t *testing.T) {
	m := withLogs(t)

	m = press(t, m, "v", "v") // WARN and above
	if rows := m.workspace.Rows(); len(rows) != 1 || rows[0].Cells[1] != "WARN" {
		t.Fatalf("rows at WARN = %v", rows)
	}

	m = press(t, m, "v", "v", "t") // back to every line, then a target
	if m.modal != modal.Input || m.inputFor != inputLogTarget {
		t.Fatalf("t opened modal %d", m.modal)
	}
	m = typeLine(t, m, "phantom_api")
	m = arrow(t, m, tea.KeyEnter)

	if rows := m.workspace.Rows(); len(rows) != 1 || rows[0].Cells[3] != "listening" {
		t.Errorf("rows for the target = %v", rows)
	}
}
