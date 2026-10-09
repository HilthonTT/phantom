package workspace

import (
	"strings"

	"charm.land/bubbles/v2/textinput"
	tea "charm.land/bubbletea/v2"

	"github.com/HilthonTT/phantom/cli/internal/tui/resource"
	"github.com/HilthonTT/phantom/cli/internal/tui/sample"
	"github.com/HilthonTT/phantom/cli/internal/tui/theme"
)

const MaxTabs = 3

const MinTabWidth = 34

type Tab struct {
	Section resource.Section

	listing resource.Listing
	cursor  int

	top int
}

// Source builds a section's listing.
type Source func(resource.Section) resource.Listing

type Model struct {
	theme  theme.Theme
	glyphs theme.Glyphs

	source Source

	tabs   []Tab
	active int

	width  int
	height int

	filter    textinput.Model
	filtering bool
}

func New(t theme.Theme, g theme.Glyphs, s resource.Section) Model {
	filter := t.Input(" / ", "filter rows", t.Palette.Surface)

	return Model{
		theme:  t,
		glyphs: g,
		source: sample.Listing,
		tabs:   []Tab{{Section: s, listing: sample.Listing(s)}},
		filter: filter,
	}
}

func (m Model) newTab(s resource.Section) Tab {
	return Tab{Section: s, listing: m.source(s)}
}

// SetSource switches where listings come from and rebuilds every open tab,
// keeping each one's cursor and the rows it had marked.
func (m *Model) SetSource(src Source) {
	m.source = src

	for i, t := range m.tabs {
		fresh := m.newTab(t.Section)
		fresh.cursor, fresh.top = t.cursor, t.top

		for j := range fresh.listing.Rows {
			for _, old := range t.listing.Rows {
				if old.Marked && old.Cells[0] == fresh.listing.Rows[j].Cells[0] {
					fresh.listing.Rows[j].Marked = true
				}
			}
		}

		m.tabs[i] = fresh
	}

	for i := range m.tabs {
		m.tabs[i].cursor = min(m.tabs[i].cursor, max(len(m.rowsOf(i))-1, 0))
	}
	m.clampScrollAll()
}

func (m *Model) SetSize(width, height int) {
	m.width, m.height = width, height
	m.filter.SetWidth(max(m.tabWidth()-10, 4))
	m.clampScrollAll()
}

func (m Model) Tabs() int { return len(m.tabs) }

func (m Model) Active() int { return m.active }

func (m Model) Section() resource.Section { return m.tabs[m.active].Section }

func (m Model) Filtering() bool { return m.filtering }

func (m Model) Rows() []resource.Row { return m.rowsOf(m.active) }

func (m Model) rowsOf(i int) []resource.Row {
	t := m.tabs[i]
	query := strings.ToLower(strings.TrimSpace(m.filter.Value()))
	if i != m.active || query == "" {
		return t.listing.Rows
	}

	var kept []resource.Row
	for _, r := range t.listing.Rows {
		if strings.Contains(strings.ToLower(strings.Join(r.Cells, " ")), query) {
			kept = append(kept, r)
		}
	}

	return kept
}

func (m Model) Selected() (resource.Row, bool) {
	rows := m.Rows()
	if len(rows) == 0 {
		return resource.Row{}, false
	}

	return rows[min(m.tabs[m.active].cursor, len(rows)-1)], true
}

// Select moves the cursor to the first row match accepts, clearing any
// filter, and reports whether there was one.
func (m *Model) Select(match func(resource.Row) bool) bool {
	m.clearFilter()

	for i, r := range m.tabs[m.active].listing.Rows {
		if match(r) {
			m.moveTo(i)
			return true
		}
	}

	return false
}

func (m *Model) Open(s resource.Section) {
	m.tabs[m.active] = m.newTab(s)
	m.clearFilter()
}

func (m *Model) OpenTab(s resource.Section) {
	if len(m.tabs) >= MaxTabs || m.width/(len(m.tabs)+1) < MinTabWidth {
		return
	}

	m.tabs = append(m.tabs, m.newTab(s))
	m.active = len(m.tabs) - 1
	m.clearFilter()
	m.SetSize(m.width, m.height)
}

func (m *Model) CloseTab() {
	if len(m.tabs) == 1 {
		return
	}

	m.tabs = append(m.tabs[:m.active], m.tabs[m.active+1:]...)
	m.active = min(m.active, len(m.tabs)-1)
	m.clearFilter()
	m.SetSize(m.width, m.height)
}

func (m *Model) NextTab() {
	m.active = (m.active + 1) % len(m.tabs)
	m.clearFilter()
	m.clampScroll()
}

func (m *Model) PrevTab() {
	m.active = (m.active - 1 + len(m.tabs)) % len(m.tabs)
	m.clearFilter()
	m.clampScroll()
}

func (m *Model) MoveUp()   { m.moveTo(m.tabs[m.active].cursor - 1) }
func (m *Model) MoveDown() { m.moveTo(m.tabs[m.active].cursor + 1) }
func (m *Model) PageUp()   { m.moveTo(m.tabs[m.active].cursor - m.rowsPerTab()) }
func (m *Model) PageDown() { m.moveTo(m.tabs[m.active].cursor + m.rowsPerTab()) }
func (m *Model) Top()      { m.moveTo(0) }
func (m *Model) Bottom()   { m.moveTo(len(m.Rows()) - 1) }

func (m *Model) moveTo(i int) {
	last := max(len(m.Rows())-1, 0)
	m.tabs[m.active].cursor = min(max(i, 0), last)
	m.clampScroll()
}

func (m *Model) ToggleMark() {
	rows := m.tabs[m.active].listing.Rows
	visible := m.Rows()
	if len(visible) == 0 {
		return
	}

	target := visible[m.tabs[m.active].cursor]
	for i := range rows {
		if sameRow(rows[i], target) {
			rows[i].Marked = !rows[i].Marked
			return
		}
	}
}

func (m *Model) MarkAll()    { m.setMarks(true) }
func (m *Model) ClearMarks() { m.setMarks(false) }

func (m *Model) setMarks(marked bool) {
	rows := m.tabs[m.active].listing.Rows
	for i := range rows {
		rows[i].Marked = marked
	}
}

func marks(t Tab) int {
	n := 0
	for _, r := range t.listing.Rows {
		if r.Marked {
			n++
		}
	}

	return n
}

func (m *Model) Reload() {
	cursor := m.tabs[m.active].cursor
	m.tabs[m.active] = m.newTab(m.tabs[m.active].Section)
	m.moveTo(cursor)
}

func (m *Model) StartFiltering() tea.Cmd {
	m.filtering = true
	return m.filter.Focus()
}

func (m *Model) StopFiltering() {
	m.clearFilter()
	m.tabs[m.active].cursor = 0
	m.tabs[m.active].top = 0
}

func (m *Model) UpdateFilter(msg tea.Msg) tea.Cmd {
	var cmd tea.Cmd
	m.filter, cmd = m.filter.Update(msg)
	m.moveTo(m.tabs[m.active].cursor)

	return cmd
}

func (m *Model) clearFilter() {
	m.filtering = false
	m.filter.Blur()
	m.filter.SetValue("")
}

func (m *Model) clampScroll() { m.clampScrollAt(m.active) }

func (m *Model) clampScrollAll() {
	for i := range m.tabs {
		m.clampScrollAt(i)
	}
}

func (m *Model) clampScrollAt(i int) {
	t := &m.tabs[i]
	perTab := m.rowsPerTab()

	if t.cursor < t.top {
		t.top = t.cursor
	}
	if t.cursor >= t.top+perTab {
		t.top = t.cursor - perTab + 1
	}

	t.top = min(max(t.top, 0), max(len(m.rowsOf(i))-perTab, 0))
}

func sameRow(a, b resource.Row) bool {
	return strings.Join(a.Cells, "\x00") == strings.Join(b.Cells, "\x00")
}
