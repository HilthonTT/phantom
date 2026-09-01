package app

import (
	"strings"
	"testing"

	tea "charm.land/bubbletea/v2"
	"charm.land/lipgloss/v2"

	"github.com/HilthonTT/phantom/cli/internal/tui/modal"
	"github.com/HilthonTT/phantom/cli/internal/tui/resource"
	"github.com/HilthonTT/phantom/cli/internal/tui/workspace"
)

// sized returns the interface laid out for a terminal of the given size.
func sized(t *testing.T, width, height int) Model {
	t.Helper()

	next, _ := New().Update(tea.WindowSizeMsg{Width: width, Height: height})

	m, ok := next.(Model)
	if !ok {
		t.Fatalf("Update returned %T, want app.Model", next)
	}

	return m
}

// press feeds keystrokes to the interface, as the terminal would.
//
// A key is named the way [tea.KeyPressMsg.String] would print it — "j", "L",
// "esc" — which is also what the bindings are written against. Putting the
// name in Text is what makes String report it back.
func press(t *testing.T, m Model, keys ...string) Model {
	t.Helper()

	for _, k := range keys {
		next, _ := m.Update(tea.KeyPressMsg{Code: rune(k[0]), Text: k})
		var ok bool
		if m, ok = next.(Model); !ok {
			t.Fatalf("Update returned %T, want app.Model", next)
		}
	}

	return m
}

// The layout is a grid of fixed rectangles. If any one of them is a column or
// a row out, every panel to the right of it or below it is pushed askew, so
// the whole render has to come out exactly the size of the terminal.
func TestLayoutFillsTheTerminalExactly(t *testing.T) {
	sizes := []struct{ width, height int }{
		{80, 24},
		{96, 26},
		{110, 30},
		{140, 40},
		{240, 60},
		{81, 25},
		{131, 41},
	}

	for _, s := range sizes {
		m := sized(t, s.width, s.height)
		out := m.render()

		if got := lipgloss.Width(out); got != s.width {
			t.Errorf("at %dx%d: width = %d, want %d", s.width, s.height, got, s.width)
		}
		if got := lipgloss.Height(out); got != s.height {
			t.Errorf("at %dx%d: height = %d, want %d", s.width, s.height, got, s.height)
		}
	}
}

// Opening tabs splits the workspace, and the split has to divide the width
// without losing or gaining a column to the rounding. The width is odd so that
// the division does not come out even.
func TestExtraTabsDoNotChangeTheLayoutSize(t *testing.T) {
	const width = 181

	m := sized(t, width, 34)

	for tabs := 1; tabs <= workspace.MaxTabs; tabs++ {
		out := m.render()

		if got := lipgloss.Width(out); got != width {
			t.Errorf("with %d tabs: width = %d, want %d", tabs, got, width)
		}
		if got := m.workspace.Tabs(); got != tabs {
			t.Errorf("expected %d tabs, have %d", tabs, got)
		}

		m.workspace.OpenTab(resource.Rooms)
	}
}

// A terminal too narrow to split again keeps the tabs it has rather than
// opening one too thin to read.
func TestTabsStopOpeningWhenThereIsNoRoom(t *testing.T) {
	m := sized(t, 96, 30)

	for range workspace.MaxTabs + 2 {
		m.workspace.OpenTab(resource.Rooms)
	}

	if got := m.workspace.Tabs(); got > workspace.MaxTabs {
		t.Errorf("opened %d tabs, more than the %d maximum", got, workspace.MaxTabs)
	}
	if got := lipgloss.Width(m.render()); got != 96 {
		t.Errorf("width = %d, want 96", got)
	}
}

// A modal is composited over the layout, so it must not resize it.
func TestModalsOverlayWithoutResizingTheLayout(t *testing.T) {
	for _, kind := range []modal.Kind{modal.Help, modal.Prompt, modal.Confirm} {
		m := sized(t, 120, 32)
		m.modal = kind
		m.confirm.Ask("A question", "and what it means")

		out := m.render()

		if got := lipgloss.Width(out); got != 120 {
			t.Errorf("modal %d: width = %d, want 120", kind, got)
		}
		if got := lipgloss.Height(out); got != 32 {
			t.Errorf("modal %d: height = %d, want 32", kind, got)
		}
	}
}

// Below the minimum the panels cannot hold a row, so the layout is replaced by
// a warning that still fills the terminal.
func TestTooSmallTerminalGetsAWarning(t *testing.T) {
	m := sized(t, 60, 18)
	out := m.render()

	if !strings.Contains(out, "too small") {
		t.Errorf("expected a size warning, got:\n%s", out)
	}
	if got := lipgloss.Width(out); got != 60 {
		t.Errorf("warning width = %d, want 60", got)
	}
	if got := lipgloss.Height(out); got != 18 {
		t.Errorf("warning height = %d, want 18", got)
	}
}

// The interface starts on the workspace, and `?` and `:` open their modals
// from wherever the keyboard is.
func TestHotkeysOpenTheModals(t *testing.T) {
	m := sized(t, 120, 32)
	if m.focus != focusWorkspace {
		t.Fatalf("focus = %d, want the workspace", m.focus)
	}

	m = press(t, m, "?")
	if m.modal != modal.Help {
		t.Errorf("`?` did not open the help menu")
	}

	m = press(t, m, "esc")
	if m.modal != modal.None {
		t.Errorf("esc did not dismiss the help menu")
	}

	m = press(t, m, ":")
	if m.modal != modal.Prompt {
		t.Errorf("`:` did not open the prompt")
	}
}

// Focus walks the three panels that can hold the keyboard, and comes back
// round to where it started.
func TestFocusCyclesThroughThePanels(t *testing.T) {
	m := sized(t, 120, 32)

	seen := map[focus]bool{m.focus: true}
	for range int(focusCount) - 1 {
		m = press(t, m, "L")
		seen[m.focus] = true
	}

	if len(seen) != int(focusCount) {
		t.Errorf("focus reached %d panels, want %d", len(seen), focusCount)
	}

	m = press(t, m, "L")
	if m.focus != focusWorkspace {
		t.Errorf("focus did not wrap back to the workspace, got %d", m.focus)
	}
}

// Moving down the listing moves the cursor, and the panel follows it past the
// bottom of the window rather than leaving it behind.
func TestCursorMovesAndTheListingScrolls(t *testing.T) {
	m := sized(t, 120, 32)
	m.workspace.Open(resource.Users)

	rows := len(m.workspace.Rows())
	for range rows + 5 {
		m = press(t, m, "j")
	}

	selected, ok := m.workspace.Selected()
	if !ok {
		t.Fatal("nothing selected after moving to the bottom")
	}

	last := m.workspace.Rows()[rows-1]
	if selected.Cells[0] != last.Cells[0] {
		t.Errorf("cursor stopped at %q, want the last row %q", selected.Cells[0], last.Cells[0])
	}
}

// Filtering narrows the sidebar to the sections that match, and leaving the
// filter restores all of them.
func TestSidebarFilterNarrowsTheSections(t *testing.T) {
	m := sized(t, 120, 32)
	m.focus = focusSidebar

	all := len(m.sidebar.Sections())

	m = press(t, m, "/")
	if !m.sidebar.Filtering() {
		t.Fatal("`/` did not open the sidebar filter")
	}

	m = press(t, m, "r", "o", "o")
	if got := len(m.sidebar.Sections()); got != 1 {
		t.Errorf("filtering on \"roo\" left %d sections, want 1", got)
	}

	m = press(t, m, "esc")
	if got := len(m.sidebar.Sections()); got != all {
		t.Errorf("leaving the filter left %d sections, want %d", got, all)
	}
}
