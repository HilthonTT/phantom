package app

import (
	"github.com/HilthonTT/phantom/cli/internal/tui/inspector"
	"github.com/HilthonTT/phantom/cli/internal/tui/sidebar"
)

const (
	MinWidth  = 80
	MinHeight = 24
)

const footerHeight = 8

const (
	tasksShare   = 38
	summaryShare = 30
	shareTotal   = 100
)

func (m Model) tooSmall() bool {
	return m.width < MinWidth || m.height < MinHeight
}

func (m *Model) resize(width, height int) {
	m.width, m.height = width, height
	m.help.SetSize(width, height)
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

func (m Model) showInspector() bool { return m.width >= inspector.MinLayoutWidth }

func (m Model) workspaceWidth() int {
	width := m.width - sidebar.Width
	if m.showInspector() {
		width -= inspector.Width
	}

	return max(width, 1)
}

func (m Model) footerWidths() (tasks, summary, connection int) {
	tasks = m.width * tasksShare / shareTotal
	summary = m.width * summaryShare / shareTotal

	return tasks, summary, m.width - tasks - summary
}
