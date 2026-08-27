package workspace

import (
	"fmt"

	"charm.land/lipgloss/v2"

	"github.com/HilthonTT/phantom/cli/internal/tui/panel"
	"github.com/HilthonTT/phantom/cli/internal/tui/resource"
)

// Rows above and below the listing itself: the filter box, the column header
// and the rule under it.
const chromeRows = 3

// Render draws every open tab side by side. `focused` says whether the
// workspace as a whole has the keyboard; within it, only the active tab is
// drawn as focused.
func (m Model) Render(focused bool) string {
	boxes := make([]string, 0, len(m.tabs))

	for i, tab := range m.tabs {
		boxes = append(boxes, m.renderTab(tab, focused && i == m.active, m.tabWidthAt(i)))
	}

	return lipgloss.JoinHorizontal(lipgloss.Top, boxes...)
}

func (m Model) renderTab(tab Tab, focused bool, width int) string {
	p := panel.New(m.theme.PanelConfig(width, m.height, focused))

	rows := m.rowsOf(tab)
	cursor := min(tab.cursor, max(len(rows)-1, 0))

	p.SetTitle(m.title(tab, rows, cursor))

	if m.filtering && focused {
		p.AddLine(" " + m.filter.View())
	} else {
		p.AddLine("")
	}

	cols := tab.listing.Columns
	w := widths(cols, p.ContentWidth()-marker-rightMargin)

	p.AddLine(m.theme.ColumnHeader.Render(panel.Pad("", marker) + header(cols, w)))
	p.AddDivider()

	if len(rows) == 0 {
		p.AddLine("")
		p.AddLine(m.theme.Faint.Render("   nothing to show"))
		return p.Render()
	}

	for i := tab.top; i < len(rows) && p.Remaining() > 0; i++ {
		p.AddLine(m.renderRow(rows[i], cols, w, i == cursor && focused))
	}

	p.SetInfo(fmt.Sprintf("%d/%d", cursor+1, len(rows)), m.footnote(tab))

	return p.Render()
}

// title is the section's name, and what the cursor is on after it, so a panel
// says what it is showing without the inspector having to be open.
func (m Model) title(tab Tab, rows []resource.Row, cursor int) string {
	if cursor >= len(rows) || len(rows[cursor].Cells) == 0 {
		return tab.Section.String()
	}

	return tab.Section.String() + " " + m.glyphs.Arrow + " " + rows[cursor].Cells[0]
}

// footnote is what the bottom border says beside the row count: how many rows
// are marked, or how the listing is ordered when none are.
func (m Model) footnote(tab Tab) string {
	if marked := marks(tab); marked > 0 {
		return fmt.Sprintf("%d marked", marked)
	}

	return tab.listing.Sort
}

// renderRow draws one row: the cursor and mark column, then the cells.
func (m Model) renderRow(r resource.Row, cols []resource.Column, w []int, underCursor bool) string {
	cursor := " "
	if underCursor {
		cursor = m.glyphs.Cursor
	}

	mark := " "
	if r.Marked {
		mark = m.glyphs.Marked
	}

	body := row(r.Cells, cols, w)

	style := m.theme.ForState(r.State)
	switch {
	case underCursor:
		style = m.theme.RowSelected
	case r.Marked:
		style = m.theme.RowMarked
	}

	return m.theme.Cursor.Render(" "+cursor+" "+mark+" ") + style.Render(body)
}

// rowsPerTab is how many listing rows fit in a tab at the current height.
func (m Model) rowsPerTab() int {
	return max(m.height-2-chromeRows, 1)
}

// tabWidth is the nominal width of one tab.
func (m Model) tabWidth() int {
	if len(m.tabs) == 0 {
		return m.width
	}

	return m.width / len(m.tabs)
}

// tabWidthAt is [Model.tabWidth] for tab i, with the remainder from the
// division given to the last tab so the row of them fills the width exactly.
func (m Model) tabWidthAt(i int) int {
	if i < len(m.tabs)-1 {
		return m.tabWidth()
	}

	return m.width - m.tabWidth()*(len(m.tabs)-1)
}
