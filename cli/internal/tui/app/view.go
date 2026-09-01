package app

import (
	"fmt"

	tea "charm.land/bubbletea/v2"
	"charm.land/lipgloss/v2"

	"github.com/HilthonTT/phantom/cli/internal/tui/modal"
	"github.com/HilthonTT/phantom/cli/internal/tui/panel"
)

// View draws the whole interface: the main row of panels, the footer row of
// boxes under it, and whichever modal is open composited over the top.
func (m Model) View() tea.View {
	view := tea.NewView(m.render())
	view.AltScreen = true
	view.BackgroundColor = m.theme.Palette.Canvas

	return view
}

func (m Model) render() string {
	if m.quitting {
		return ""
	}
	if m.tooSmall() {
		return m.renderTooSmall()
	}

	layout := lipgloss.JoinVertical(lipgloss.Left, m.renderMain(), m.renderFooter())

	box := m.renderModal()
	if box == "" {
		return layout
	}

	return modal.Overlay(layout, box, m.width, m.height)
}

// renderMain is the sidebar, the workspace and — where the terminal is wide
// enough — the inspector.
func (m Model) renderMain() string {
	open := m.workspace.Section()
	if m.focus == focusSidebar {
		if selected, ok := m.sidebar.Selected(); ok {
			open = selected
		}
	}

	panels := []string{
		m.sidebar.Render(m.focus == focusSidebar, open),
		m.workspace.Render(m.focus == focusWorkspace),
	}

	if m.showInspector() {
		row, ok := m.workspace.Selected()
		panels = append(panels, m.inspector.Render(m.workspace.Section(), row, ok))
	}

	return lipgloss.JoinHorizontal(lipgloss.Top, panels...)
}

// renderFooter is the tasks, summary and connection boxes.
func (m Model) renderFooter() string {
	row, ok := m.workspace.Selected()

	return lipgloss.JoinHorizontal(lipgloss.Top,
		m.taskbar.Render(m.focus == focusTasks),
		m.summary.Render(row, ok, false),
		m.connection.Render(),
	)
}

// renderModal is the open modal, or the empty string where none is.
func (m Model) renderModal() string {
	switch m.modal {
	case modal.Help:
		return m.help.Render(m.width, m.height)
	case modal.Prompt:
		return m.prompt.Render(m.width, m.height)
	case modal.Confirm:
		return m.confirm.Render(m.width, m.height)
	default:
		return ""
	}
}

// renderTooSmall is what is drawn instead of the layout when the terminal
// cannot hold it: what the terminal is, and what it would need to be.
func (m Model) renderTooSmall() string {
	dimension := func(have, need int) string {
		style := m.theme.StateDone
		if have < need {
			style = m.theme.StateFailed
		}

		return style.Render(fmt.Sprintf("%d", have)) +
			m.theme.Faint.Render(fmt.Sprintf("/%d", need))
	}

	body := lipgloss.JoinVertical(lipgloss.Left,
		m.theme.Title.Render("The terminal is too small"),
		"",
		m.theme.Muted.Render("width  ")+dimension(m.width, MinWidth),
		m.theme.Muted.Render("height ")+dimension(m.height, MinHeight),
		"",
		m.theme.Faint.Render(panel.Truncate("resize, or press q to quit", max(m.width-2, 1))),
	)

	return lipgloss.Place(m.width, m.height, lipgloss.Center, lipgloss.Center, body)
}
