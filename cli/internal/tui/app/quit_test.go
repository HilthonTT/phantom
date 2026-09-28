package app

import (
	"testing"

	tea "charm.land/bubbletea/v2"

	"github.com/HilthonTT/phantom/cli/internal/tui/modal"
)

func TestQuitAsksFirst(t *testing.T) {
	m := sized(t, 120, 32)

	m = press(t, m, "q")
	if m.quitting {
		t.Fatal("`q` quit without asking")
	}
	if m.modal != modal.Confirm {
		t.Fatalf("`q` opened modal %d, want the confirm dialog", m.modal)
	}
}

func TestQuitCanBeCancelled(t *testing.T) {
	for _, dismiss := range []rune{tea.KeyEnter, tea.KeyEscape} {
		m := sized(t, 120, 32)

		m = press(t, m, "q")
		m = arrow(t, m, dismiss)

		if m.quitting {
			t.Errorf("key %q quit, want the dialog dismissed", dismiss)
		}
		if m.modal != modal.None {
			t.Errorf("key %q left modal %d open", dismiss, m.modal)
		}
	}
}

func TestQuitConfirmed(t *testing.T) {
	m := sized(t, 120, 32)

	m = press(t, m, "q")
	m = arrow(t, m, tea.KeyTab)
	m = arrow(t, m, tea.KeyEnter)

	if !m.quitting {
		t.Error("confirming the dialog did not quit")
	}
}

func TestOtherConfirmationsDoNotQuit(t *testing.T) {
	m := sized(t, 120, 32)

	m = press(t, m, "s")
	m = arrow(t, m, tea.KeyTab)
	m = arrow(t, m, tea.KeyEnter)

	if m.quitting {
		t.Error("confirming the sort dialog quit phantom")
	}
}

func TestCtrlCQuitsAtOnce(t *testing.T) {
	m := openChat(t, 120, 32)
	m = arrow(t, m, tea.KeyEnter)

	next, _ := m.Update(tea.KeyPressMsg{Code: 'c', Mod: tea.ModCtrl})
	if !next.(Model).quitting {
		t.Error("ctrl+c while composing did not quit")
	}
}

func TestQuitIsImmediateWhenTheTerminalIsTooSmall(t *testing.T) {
	m := sized(t, 60, 18)

	m = press(t, m, "q")
	if !m.quitting {
		t.Error("`q` on the too-small screen did not quit")
	}
}
