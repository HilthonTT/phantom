package app

import (
	"github.com/HilthonTT/phantom/cli/internal/tui/inspector"
	"github.com/HilthonTT/phantom/cli/internal/tui/sidebar"
)

// The smallest terminal the interface is drawn in. Below either of these the
// panels cannot hold a row between their borders, so a warning is drawn
// instead of a layout that would be unreadable.
const (
	MinWidth  = 80
	MinHeight = 24
)

// footerHeight is how many lines the row of footer boxes takes, borders
// included: a title line and two content lines is the least that shows a task
// with its progress bar.
const footerHeight = 8

// The footer's three boxes divide the width in these proportions. The tasks
// box is widest because it is the only one holding a progress bar.
const (
	tasksShare   = 38
	summaryShare = 30
	shareTotal   = 100
)

// tooSmall reports whether the terminal is below [MinWidth] or [MinHeight].
func (m Model) tooSmall() bool {
	return m.width < MinWidth || m.height < MinHeight
}

// resize hands every panel its share of the terminal.
//
// The sidebar and the inspector are fixed widths and the workspace takes what
// is left, so widening the terminal widens the listings rather than the
// furniture around them. The inspector is dropped entirely on a narrow
// terminal, where those columns are better spent on the listing.
func (m *Model) resize(width, height int) {
	m.width, m.height = width, height
	if m.tooSmall() {
		return
	}

	mainHeight := height - footerHeight

	m.sidebar.SetHeight(mainHeight)
	m.inspector.SetHeight(mainHeight)
	m.workspace.SetSize(m.workspaceWidth(), mainHeight)

	tasks, summary, conn := m.footerWidths()
	m.taskbar.SetSize(tasks, footerHeight)
	m.summary.SetSize(summary, footerHeight)
	m.connection.SetSize(conn, footerHeight)
}

// showInspector reports whether the terminal is wide enough to spare the
// inspector's columns.
func (m Model) showInspector() bool { return m.width >= inspector.MinLayoutWidth }

// workspaceWidth is whatever the sidebar and the inspector leave.
func (m Model) workspaceWidth() int {
	width := m.width - sidebar.Width
	if m.showInspector() {
		width -= inspector.Width
	}

	return max(width, 1)
}

// footerWidths splits the width between the three footer boxes, with the
// remainder from the division going to the last so the row fills the terminal
// exactly.
func (m Model) footerWidths() (tasks, summary, connection int) {
	tasks = m.width * tasksShare / shareTotal
	summary = m.width * summaryShare / shareTotal

	return tasks, summary, m.width - tasks - summary
}
