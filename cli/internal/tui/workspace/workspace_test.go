package workspace

import (
	"testing"

	"github.com/HilthonTT/phantom/cli/internal/tui/resource"
	"github.com/HilthonTT/phantom/cli/internal/tui/theme"
)

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

func TestSortCyclesThroughColumnsAndBack(t *testing.T) {
	m := New(theme.Default(), theme.Glyphs{}, resource.Users)
	m.SetSize(120, 20)
	m.SetSource(func(resource.Section) resource.Listing {
		return resource.Listing{
			Sort:    "as given",
			Columns: []resource.Column{{Title: "Name"}, {Title: "Seen"}},
			Rows: []resource.Row{
				{Cells: []string{"b", "1 hour ago"}, SortKeys: map[int]string{1: "100"}},
				{Cells: []string{"a", "2 min ago"}, SortKeys: map[int]string{1: "900"}},
				{Cells: []string{"C", "never"}, SortKeys: map[int]string{1: "0"}},
			},
		}
	})

	order := func() string {
		s := ""
		for _, r := range m.Rows() {
			s += r.Cells[0]
		}
		return s
	}

	steps := []struct{ order, label string }{
		{"abC", "name, ascending"},
		{"Cba", "name, descending"},
		{"Cba", "seen, ascending"},
		{"abC", "seen, descending"},
		{"baC", "as given"},
	}
	for i, step := range steps {
		m.CycleSort()
		if got := order(); got != step.order {
			t.Errorf("step %d: order = %s, want %s", i, got, step.order)
		}
		if got := m.tabs[m.active].sortLabel(); got != step.label {
			t.Errorf("step %d: label = %q, want %q", i, got, step.label)
		}
	}
}
