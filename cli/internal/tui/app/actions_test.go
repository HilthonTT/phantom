package app

import (
	"errors"
	"testing"

	tea "charm.land/bubbletea/v2"

	"github.com/HilthonTT/phantom/cli/internal/tui/live"
	"github.com/HilthonTT/phantom/cli/internal/tui/modal"
	"github.com/HilthonTT/phantom/cli/internal/tui/resource"
)

// withUsers is an admin looking at a users table holding bob.
func withUsers(t *testing.T) Model {
	t.Helper()

	m := signedInAs(t, true)
	listing := resource.Listing{
		Columns: []resource.Column{{Title: "User"}, {Title: "Admin"}},
		Rows:    []resource.Row{{Cells: []string{"@bob:test", "no"}, Ref: []string{"@bob:test", "active"}}},
	}
	m, _ = feed(t, m, live.AdminMsg{Gen: m.live.Admin.Gen, Section: resource.Users, Listing: listing})
	m.focus = focusWorkspace

	return m
}

func TestEnterOffersTheRowsActions(t *testing.T) {
	m := arrow(t, withUsers(t), tea.KeyEnter)

	if m.modal != modal.Menu || len(m.actions) != 4 {
		t.Fatalf("modal = %d with %d actions, want the menu of 4", m.modal, len(m.actions))
	}
}

func TestADestructiveChoiceIsConfirmedThenRun(t *testing.T) {
	m := arrow(t, withUsers(t), tea.KeyEnter)
	m = arrow(t, m, tea.KeyDown)
	m = arrow(t, m, tea.KeyDown) // Deactivate

	m = arrow(t, m, tea.KeyEnter)
	if m.modal != modal.Confirm || m.acting.Kind != live.DeactivateUser {
		t.Fatalf("modal = %d acting %v, want the confirm for deactivating", m.modal, m.acting.Label())
	}

	m = arrow(t, m, tea.KeyTab)
	next, cmd := m.Update(tea.KeyPressMsg{Code: tea.KeyEnter})
	m = next.(Model)
	if cmd == nil || m.modal != modal.None {
		t.Fatal("confirming ran nothing")
	}
}

func TestAnActionsOutcomeIsShown(t *testing.T) {
	m := withUsers(t)
	a := live.Action{Kind: live.GrantAdmin, Target: "@bob:test"}

	m, cmd := feed(t, m, live.ActedMsg{Action: a, Done: "@bob:test is now an admin."})
	if m.modal != modal.Notice || m.notice.Render(80, 24) == "" || cmd == nil {
		t.Error("a done action showed no notice, or refreshed nothing")
	}

	m = arrow(t, m, tea.KeyEnter)
	m, _ = feed(t, m, live.ActedMsg{Action: a, Err: errors.New("M_FORBIDDEN")})
	if m.modal != modal.Notice {
		t.Error("a failed action showed no notice")
	}
}

func TestPromptCommandsExpandBareNames(t *testing.T) {
	m := withUsers(t)

	next, _ := m.runCommand("deactivate bob")
	m = next.(Model)
	if m.modal != modal.Confirm || m.acting.Target != "@bob:test" {
		t.Errorf("modal = %d target %q, want the confirm for @bob:test", m.modal, m.acting.Target)
	}
}

func TestPromptRunsAnEmptyOrCapitalisedLine(t *testing.T) {
	m := withUsers(t)

	for _, line := range []string{"", "   "} {
		next, _ := m.runCommand(line)
		if next.(Model).modal != modal.None {
			t.Errorf("runCommand(%q) opened modal %d, want none", line, next.(Model).modal)
		}
	}

	next, _ := m.runCommand("Deactivate bob")
	if got := next.(Model); got.modal != modal.Confirm || got.acting.Target != "@bob:test" {
		t.Errorf("modal = %d target %q, want the confirm for @bob:test", got.modal, got.acting.Target)
	}
}

func TestPromptAdminCommandsNeedAnAdmin(t *testing.T) {
	m := signedInAs(t, false)

	next, _ := m.runCommand("backup")
	m = next.(Model)
	if m.modal != modal.Notice {
		t.Errorf("modal = %d, want the admins-only notice", m.modal)
	}
}

func TestTokenArgs(t *testing.T) {
	a, err := tokenArgs(live.Action{Kind: live.CreateToken}, []string{"5", "7", "welcome"})
	if err != nil || a.Uses != 5 || a.Days != 7 || a.Secret != "welcome" {
		t.Errorf("tokenArgs = %+v, %v", a, err)
	}
	if _, err := tokenArgs(live.Action{}, []string{"-1"}); err == nil {
		t.Error("negative uses accepted")
	}
	if _, err := tokenArgs(live.Action{}, []string{"a", "b"}); err == nil {
		t.Error("two tokens accepted")
	}
}
