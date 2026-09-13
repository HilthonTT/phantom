package workspace

import (
	"testing"

	"github.com/HilthonTT/phantom/cli/internal/tui/resource"
	"github.com/HilthonTT/phantom/cli/internal/tui/theme"
)

// twoTabs is a workspace of the given size with a second tab open and the
// first tab scrolled to the end of its listing, which is the state a resize
// has to cope with.
func twoTabs(t *testing.T, width, height int) Model {
	t.Helper()

	m := New(theme.Default(), theme.Glyphs{}, resource.Services)
	m.SetSize(width, height)
	m.OpenTab(resource.Rooms)

	m.PrevTab()
	m.Bottom()
	m.NextTab()

	return m
}

// Every tab is on screen at once, so every tab's window has to be refitted
// when the terminal grows. One left where a shorter panel put it draws its
// last few rows against a run of blank lines, with the rows above the window
// unreachable until the keyboard next reaches that tab.
func TestResizeRefitsEveryTabsWindow(t *testing.T) {
	m := twoTabs(t, 120, 12)

	if m.tabs[0].top == 0 {
		t.Fatal("tab 0 did not scroll in a short panel, so the test proves nothing")
	}

	m.SetSize(120, 40)

	for i := range m.tabs {
		want := max(len(m.rowsOf(i))-m.rowsPerTab(), 0)
		if got := m.tabs[i].top; got > want {
			t.Errorf("tab %d: top = %d after the panel grew, want at most %d", i, got, want)
		}
	}
}

// Shrinking must not leave a cursor outside the window that is drawn for it.
func TestResizeKeepsEveryCursorInItsWindow(t *testing.T) {
	m := twoTabs(t, 120, 40)

	m.SetSize(120, 14)

	perTab := m.rowsPerTab()
	for i := range m.tabs {
		top, cursor := m.tabs[i].top, m.tabs[i].cursor
		if cursor < top || cursor >= top+perTab {
			t.Errorf("tab %d: cursor %d outside window [%d,%d)", i, cursor, top, top+perTab)
		}
	}
}
