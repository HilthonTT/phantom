package app

import (
	"fmt"

	tea "charm.land/bubbletea/v2"
	"charm.land/lipgloss/v2"

	"github.com/HilthonTT/phantom/cli/internal/tui/modal"
	"github.com/HilthonTT/phantom/cli/internal/tui/panel"
)

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

func (m Model) renderFooter() string {
	row, ok := m.workspace.Selected()

	return lipgloss.JoinHorizontal(lipgloss.Top,
		m.taskbar.Render(m.focus == focusTasks),
		m.summary.Render(row, ok, false),
		m.connection.Render(),
	)
}

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
