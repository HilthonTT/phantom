package app

import (
	"charm.land/bubbles/v2/key"
	tea "charm.land/bubbletea/v2"

	"github.com/HilthonTT/phantom/cli/internal/tui/modal"
)

// Update routes a message to whatever currently has the keyboard.
//
// The order is the priority order: a modal takes everything, then a filter box
// takes everything that is not a way out of it, then the global keys, then the
// focused panel.
func (m Model) Update(msg tea.Msg) (tea.Model, tea.Cmd) {
	switch msg := msg.(type) {
	case tea.WindowSizeMsg:
		m.resize(msg.Width, msg.Height)
		return m, nil

	case tea.KeyPressMsg:
		return m.handleKey(msg)
	}

	return m, nil
}

func (m Model) handleKey(msg tea.KeyPressMsg) (tea.Model, tea.Cmd) {
	if m.modal != modal.None {
		return m.handleModalKey(msg)
	}
	if m.filtering() {
		return m.handleFilterKey(msg)
	}
	if handled, model, cmd := m.handleGlobalKey(msg); handled {
		return model, cmd
	}

	return m.handlePanelKey(msg)
}

// handleGlobalKey handles the keys that mean the same thing wherever the
// keyboard is. The bool says whether the key was one of them.
func (m Model) handleGlobalKey(msg tea.KeyPressMsg) (bool, tea.Model, tea.Cmd) {
	switch {
	case key.Matches(msg, m.keys.Quit):
		m.quitting = true
		return true, m, tea.Quit

	case key.Matches(msg, m.keys.Help):
		m.modal = modal.Help
		return true, m, m.help.Focus()

	case key.Matches(msg, m.keys.Prompt):
		m.modal = modal.Prompt
		return true, m, m.prompt.Focus()

	case key.Matches(msg, m.keys.FocusNext):
		m.focus = (m.focus + 1) % focusCount
		return true, m, nil

	case key.Matches(msg, m.keys.FocusPrev):
		m.focus = (m.focus - 1 + focusCount) % focusCount
		return true, m, nil

	case key.Matches(msg, m.keys.Filter):
		model, cmd := m.startFiltering()
		return true, model, cmd

	case key.Matches(msg, m.keys.Sort):
		m.confirm.Ask("Change the sort order",
			"Sorting is not wired up yet — this is where it will ask.")
		m.modal = modal.Confirm
		return true, m, nil
	}

	return false, m, nil
}

// startFiltering opens the filter box of whichever panel has the keyboard.
func (m Model) startFiltering() (tea.Model, tea.Cmd) {
	switch m.focus {
	case focusSidebar:
		return m, m.sidebar.StartFiltering()
	case focusWorkspace:
		return m, m.workspace.StartFiltering()
	default:
		return m, nil
	}
}

// handleFilterKey routes to the open filter box, intercepting only the keys
// that close it or step the cursor while it stays open.
func (m Model) handleFilterKey(msg tea.KeyPressMsg) (tea.Model, tea.Cmd) {
	switch {
	case key.Matches(msg, m.keys.Cancel):
		m.sidebar.StopFiltering()
		m.workspace.StopFiltering()
		return m, nil

	case key.Matches(msg, m.keys.Up):
		return m, nil

	case key.Matches(msg, m.keys.Down):
		return m, nil
	}

	if m.sidebar.Filtering() {
		return m, m.sidebar.UpdateFilter(msg)
	}

	return m, m.workspace.UpdateFilter(msg)
}

// handlePanelKey routes to the focused panel.
func (m Model) handlePanelKey(msg tea.KeyPressMsg) (tea.Model, tea.Cmd) {
	switch m.focus {
	case focusSidebar:
		return m.handleSidebarKey(msg), nil
	case focusWorkspace:
		return m.handleWorkspaceKey(msg), nil
	default:
		return m.handleTaskbarKey(msg), nil
	}
}

func (m Model) handleSidebarKey(msg tea.KeyPressMsg) tea.Model {
	switch {
	case key.Matches(msg, m.keys.Up):
		m.sidebar.MoveUp()

	case key.Matches(msg, m.keys.Down):
		m.sidebar.MoveDown()

	case key.Matches(msg, m.keys.Open):
		if section, ok := m.sidebar.Selected(); ok {
			m.workspace.Open(section)
			m.focus = focusWorkspace
		}

	case key.Matches(msg, m.keys.OpenPanel):
		if section, ok := m.sidebar.Selected(); ok {
			m.workspace.OpenTab(section)
			m.focus = focusWorkspace
		}
	}

	return m
}

func (m Model) handleWorkspaceKey(msg tea.KeyPressMsg) tea.Model {
	switch {
	case key.Matches(msg, m.keys.Up):
		m.workspace.MoveUp()
	case key.Matches(msg, m.keys.Down):
		m.workspace.MoveDown()
	case key.Matches(msg, m.keys.PageUp):
		m.workspace.PageUp()
	case key.Matches(msg, m.keys.PageDown):
		m.workspace.PageDown()
	case key.Matches(msg, m.keys.Top):
		m.workspace.Top()
	case key.Matches(msg, m.keys.Bottom):
		m.workspace.Bottom()

	case key.Matches(msg, m.keys.NextPanel):
		m.workspace.NextTab()
	case key.Matches(msg, m.keys.PrevPanel):
		m.workspace.PrevTab()
	case key.Matches(msg, m.keys.OpenPanel):
		m.workspace.OpenTab(m.workspace.Section())
	case key.Matches(msg, m.keys.ClosePanel):
		m.workspace.CloseTab()

	case key.Matches(msg, m.keys.Mark):
		m.workspace.ToggleMark()
	case key.Matches(msg, m.keys.MarkAll):
		m.workspace.MarkAll()
	case key.Matches(msg, m.keys.ClearMark):
		m.workspace.ClearMarks()
	case key.Matches(msg, m.keys.Refresh):
		m.workspace.Reload()
	}

	return m
}

func (m Model) handleTaskbarKey(msg tea.KeyPressMsg) tea.Model {
	switch {
	case key.Matches(msg, m.keys.Up):
		m.taskbar.MoveUp()

	case key.Matches(msg, m.keys.Down):
		m.taskbar.MoveDown()

	case key.Matches(msg, m.keys.Cancel):
		if task, ok := m.taskbar.Selected(); ok {
			m.confirm.Ask("Cancel "+task.Name+"?",
				"Cancelling is not wired up yet — this is where it will ask.")
			m.modal = modal.Confirm
		}
	}

	return m
}

// handleModalKey routes to the open modal. Every modal closes on the cancel
// key, and none of them does anything on confirm yet.
func (m Model) handleModalKey(msg tea.KeyPressMsg) (tea.Model, tea.Cmd) {
	if key.Matches(msg, m.keys.Cancel) {
		return m.closeModal(), nil
	}

	switch m.modal {
	case modal.Help:
		return m.handleHelpKey(msg)

	case modal.Prompt:
		if key.Matches(msg, m.keys.Open) {
			return m.closeModal(), nil
		}
		return m, m.prompt.Update(msg)

	case modal.Confirm:
		switch {
		case key.Matches(msg, m.keys.NextPanel), key.Matches(msg, m.keys.PrevPanel):
			m.confirm.Toggle()
		case key.Matches(msg, m.keys.Open):
			return m.closeModal(), nil
		}
		return m, nil

	default:
		return m, nil
	}
}

func (m Model) handleHelpKey(msg tea.KeyPressMsg) (tea.Model, tea.Cmd) {
	switch {
	case key.Matches(msg, m.keys.Up):
		m.help.MoveUp()
		return m, nil

	case key.Matches(msg, m.keys.Down):
		m.help.MoveDown()
		return m, nil
	}

	return m, m.help.Update(msg)
}

func (m Model) closeModal() Model {
	m.help.Blur()
	m.prompt.Blur()
	m.modal = modal.None

	return m
}
