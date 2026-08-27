// Package workspace draws the resource listings that fill the middle of the
// interface.
//
// One listing occupies one tab, and several tabs can be open side by side so
// that two sections — the rooms in one, the users in another — can be read
// against each other. This is superfile's several-file-panels idea applied to
// an admin console: the panels are peers, one of them has the keyboard, and
// the rest keep their cursors where they were left.
package workspace

import (
	"strings"

	"charm.land/bubbles/v2/textinput"
	tea "charm.land/bubbletea/v2"

	"github.com/HilthonTT/phantom/cli/internal/tui/resource"
	"github.com/HilthonTT/phantom/cli/internal/tui/sample"
	"github.com/HilthonTT/phantom/cli/internal/tui/theme"
)

// MaxTabs is how many listings may be open at once. Past this each is too
// narrow to show a row without eliding most of it.
const MaxTabs = 3

// MinTabWidth is the narrowest a tab is drawn at; below it the layout stops
// opening new ones.
const MinTabWidth = 34

// Tab is one open listing and where its cursor is.
type Tab struct {
	Section resource.Section

	listing resource.Listing
	cursor  int

	// top is the first row drawn, which trails the cursor as it walks past the
	// bottom of the panel.
	top int
}

// Model is every open tab.
type Model struct {
	theme  theme.Theme
	glyphs theme.Glyphs

	tabs   []Tab
	active int

	width  int
	height int

	filter    textinput.Model
	filtering bool
}

// New opens a single tab on the given section.
func New(t theme.Theme, g theme.Glyphs, s resource.Section) Model {
	filter := t.Input(" / ", "filter rows", t.Palette.Surface)

	return Model{
		theme:  t,
		glyphs: g,
		tabs:   []Tab{newTab(s)},
		filter: filter,
	}
}

func newTab(s resource.Section) Tab {
	return Tab{Section: s, listing: sample.Listing(s)}
}

// SetSize sets the whole workspace's extent, borders included. The tabs split
// it between them.
func (m *Model) SetSize(width, height int) {
	m.width, m.height = width, height
	m.filter.SetWidth(max(m.tabWidth()-10, 4))
	m.clampScroll()
}

// Tabs is how many listings are open.
func (m Model) Tabs() int { return len(m.tabs) }

// Active is the index of the tab with the keyboard.
func (m Model) Active() int { return m.active }

// Section is the section the active tab is showing.
func (m Model) Section() resource.Section { return m.tabs[m.active].Section }

// Filtering reports whether the filter box has the keyboard.
func (m Model) Filtering() bool { return m.filtering }

// Rows are the active tab's rows after the filter, in display order.
func (m Model) Rows() []resource.Row { return m.rowsOf(m.tabs[m.active]) }

func (m Model) rowsOf(t Tab) []resource.Row {
	query := strings.ToLower(strings.TrimSpace(m.filter.Value()))
	if query == "" {
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

// Selected is the row under the active tab's cursor, and whether there is one.
func (m Model) Selected() (resource.Row, bool) {
	rows := m.Rows()
	if len(rows) == 0 {
		return resource.Row{}, false
	}

	return rows[min(m.tabs[m.active].cursor, len(rows)-1)], true
}

// Open replaces the active tab's listing with another section's.
func (m *Model) Open(s resource.Section) {
	m.tabs[m.active] = newTab(s)
	m.clearFilter()
}

// OpenTab adds a tab beside the others and focuses it, up to [MaxTabs] and as
// long as the result would still be wide enough to read.
func (m *Model) OpenTab(s resource.Section) {
	if len(m.tabs) >= MaxTabs || m.width/(len(m.tabs)+1) < MinTabWidth {
		return
	}

	m.tabs = append(m.tabs, newTab(s))
	m.active = len(m.tabs) - 1
	m.clearFilter()
	m.SetSize(m.width, m.height)
}

// CloseTab closes the active tab. The last one is never closed: a workspace
// with no listing in it has nothing to draw and no way back.
func (m *Model) CloseTab() {
	if len(m.tabs) == 1 {
		return
	}

	m.tabs = append(m.tabs[:m.active], m.tabs[m.active+1:]...)
	m.active = min(m.active, len(m.tabs)-1)
	m.clearFilter()
	m.SetSize(m.width, m.height)
}

// NextTab and PrevTab move the keyboard between open tabs, wrapping.
func (m *Model) NextTab() {
	m.active = (m.active + 1) % len(m.tabs)
	m.clearFilter()
}

func (m *Model) PrevTab() {
	m.active = (m.active - 1 + len(m.tabs)) % len(m.tabs)
	m.clearFilter()
}

// MoveUp, MoveDown, PageUp, PageDown, Top and Bottom walk the active tab's
// cursor. All of them stop at the ends of the listing.
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

// ToggleMark marks or unmarks the row under the cursor.
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

// MarkAll marks every row of the active tab, and ClearMarks unmarks them.
func (m *Model) MarkAll()    { m.setMarks(true) }
func (m *Model) ClearMarks() { m.setMarks(false) }

func (m *Model) setMarks(marked bool) {
	rows := m.tabs[m.active].listing.Rows
	for i := range rows {
		rows[i].Marked = marked
	}
}

// marks is how many of a tab's rows are marked.
func marks(t Tab) int {
	n := 0
	for _, r := range t.listing.Rows {
		if r.Marked {
			n++
		}
	}

	return n
}

// Reload throws away the active tab's listing and asks for it again. With no
// homeserver behind it this only resets the cursor, which is what it would do
// anyway.
func (m *Model) Reload() {
	cursor := m.tabs[m.active].cursor
	m.tabs[m.active] = newTab(m.tabs[m.active].Section)
	m.moveTo(cursor)
}

// StartFiltering gives the filter box the keyboard.
func (m *Model) StartFiltering() tea.Cmd {
	m.filtering = true
	return m.filter.Focus()
}

// StopFiltering takes the keyboard back and empties the box.
func (m *Model) StopFiltering() { m.clearFilter() }

// UpdateFilter feeds a message to the filter box.
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
	m.tabs[m.active].cursor = 0
	m.tabs[m.active].top = 0
}

// clampScroll keeps the window of drawn rows over the cursor.
//
// The last clamp is what handles the panel growing: a window that was scrolled
// to the bottom of a short panel would otherwise stay there when the terminal
// is made taller, leaving blank lines under the last row and rows hidden above
// the first.
func (m *Model) clampScroll() {
	t := &m.tabs[m.active]
	perTab := m.rowsPerTab()

	if t.cursor < t.top {
		t.top = t.cursor
	}
	if t.cursor >= t.top+perTab {
		t.top = t.cursor - perTab + 1
	}

	t.top = min(max(t.top, 0), max(len(m.Rows())-perTab, 0))
}

// sameRow identifies a row by its cells, which is enough while the rows come
// from a static table.
func sameRow(a, b resource.Row) bool {
	return strings.Join(a.Cells, "\x00") == strings.Join(b.Cells, "\x00")
}
