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

func TestSortLeavesTheSourceAndOtherTabsAlone(t *testing.T) {
	// One listing handed to every tab, as the live state does.
	shared := resource.Listing{
		Columns: []resource.Column{{Title: "Name"}},
		Rows: []resource.Row{
			{Cells: []string{"b"}}, {Cells: []string{"c"}}, {Cells: []string{"a"}},
		},
	}

	m := New(theme.Default(), theme.Glyphs{}, resource.Users)
	m.SetSize(120, 20)
	m.SetSource(func(resource.Section) resource.Listing { return shared })
	m.OpenTab(resource.Users)

	m.CycleSort()
	m.ToggleMark()

	order := func(rows []resource.Row) string {
		s := ""
		for _, r := range rows {
			s += r.Cells[0]
			if r.Marked {
				s += "*"
			}
		}
		return s
	}

	if got := order(shared.Rows); got != "bca" {
		t.Errorf("source rows = %s after sorting a tab, want bca", got)
	}
	if got := order(m.rowsOf(0)); got != "bca" {
		t.Errorf("the other tab's rows = %s, want bca", got)
	}
	if got := order(m.Rows()); got != "ab*c" {
		t.Errorf("the sorted tab's rows = %s, want ab*c", got)
	}
}

func TestRefreshKeepsTheCursorOnItsRecord(t *testing.T) {
	rows := func(names ...string) resource.Listing {
		l := resource.Listing{Columns: []resource.Column{{Title: "Name"}, {Title: "Seen"}}}
		for _, n := range names {
			l.Rows = append(l.Rows, resource.Row{Cells: []string{n, "now"}, Ref: []string{"@" + n}})
		}
		return l
	}

	m := New(theme.Default(), theme.Glyphs{}, resource.Users)
	m.SetSize(120, 20)
	m.SetSource(func(resource.Section) resource.Listing { return rows("alice", "bob", "carol") })
	m.Select(func(r resource.Row) bool { return r.Cells[0] == "bob" })

	// A refresh reorders the users, as sorting by last seen does.
	m.SetSource(func(resource.Section) resource.Listing { return rows("carol", "dave", "alice", "bob") })
	if got, _ := m.Selected(); got.Cells[0] != "bob" {
		t.Errorf("selected %q after the refresh, want bob", got.Cells[0])
	}

	// A record that went away leaves the cursor where it was, in range.
	m.SetSource(func(resource.Section) resource.Listing { return rows("alice") })
	if got, ok := m.Selected(); !ok || got.Cells[0] != "alice" {
		t.Errorf("selected %q after bob went, want alice", got.Cells[0])
	}
}
