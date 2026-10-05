package app

import (
	"strings"

	"charm.land/bubbles/v2/key"
	tea "charm.land/bubbletea/v2"

	"github.com/HilthonTT/phantom/cli/internal/tui/live"
	"github.com/HilthonTT/phantom/cli/internal/tui/modal"
	"github.com/HilthonTT/phantom/cli/internal/tui/resource"
)

func (m Model) Update(msg tea.Msg) (tea.Model, tea.Cmd) {
	switch msg := msg.(type) {
	case tea.WindowSizeMsg:
		m.resize(msg.Width, msg.Height)
		return m, nil

	case tea.KeyPressMsg:
		return m.handleKey(msg)

	case live.TickMsg:
		return m, live.Probe(m.client, true)

	case live.ProbedMsg:
		return m.probed(msg)

	case live.AuthMsg:
		return m.authed(msg)

	case live.LoggedOutMsg:
		m.live = m.live.SignOut()
		m.connection.SetServer(m.live.Server())
		return m, nil
	}

	return m, nil
}

// authed takes a login, or the check of a saved session, finishing.
func (m Model) authed(msg live.AuthMsg) (tea.Model, tea.Cmd) {
	m.live = m.live.SignIn(msg)
	m.connection.SetServer(m.live.Server())

	switch {
	case msg.Refused && msg.Resumed:
		_ = m.store.Forget(m.client.URL())
		m.saved = nil
		return m.openLogin("the saved session has ended; sign in again")

	case msg.Refused:
		if m.modal != modal.Login {
			return m, nil
		}
		return m, m.login.Fail(msg.Err.Error())

	case !msg.Resumed:
		if err := m.store.Save(m.client.URL(), msg.Session); err != nil {
			m.live.Account.Err = err
			m.connection.SetServer(m.live.Server())
		}
		if m.modal == modal.Login {
			m = m.closeModal()
		}
	}

	return m, nil
}

func (m Model) openLogin(reason string) (tea.Model, tea.Cmd) {
	m.loginOffered = true
	m.help.Blur()
	m.prompt.Blur()
	m.modal = modal.Login

	return m, m.login.Open(m.live.Host, reason)
}

func (m Model) probed(msg live.ProbedMsg) (tea.Model, tea.Cmd) {
	prev := m.live
	m.live = m.live.Apply(msg)
	m.connection.SetServer(m.live.Server())

	if m.live.Differs(prev) {
		m.workspace.SetSource(m.live.Listing)
	}

	var cmds []tea.Cmd
	if msg.Scheduled {
		cmds = append(cmds, live.Tick())
	}

	// A reachable server with no session asks for one, once; :login opens the
	// form again after it is dismissed.
	if m.live.Link == live.Connected && m.saved == nil && !m.live.Account.SignedIn() &&
		!m.loginOffered && m.modal == modal.None {
		next, cmd := m.openLogin("")
		m = next.(Model)
		cmds = append(cmds, cmd)
	}

	return m, tea.Batch(cmds...)
}

func (m Model) handleKey(msg tea.KeyPressMsg) (tea.Model, tea.Cmd) {
	if key.Matches(msg, m.keys.ForceQuit) {
		return m.quit()
	}
	if m.modal != modal.None {
		return m.handleModalKey(msg)
	}
	if m.chat.Composing() {
		return m.handleComposeKey(msg)
	}
	if m.filtering() {
		return m.handleFilterKey(msg)
	}
	if handled, model, cmd := m.handleGlobalKey(msg); handled {
		return model, cmd
	}

	return m.handlePanelKey(msg)
}

func (m Model) handleGlobalKey(msg tea.KeyPressMsg) (bool, tea.Model, tea.Cmd) {
	switch {
	case key.Matches(msg, m.keys.Quit):
		if m.tooSmall() {
			model, cmd := m.quit()
			return true, model, cmd
		}
		m.ask(quitAction, "Quit phantom?", "The session ends and the terminal is handed back.")
		return true, m, nil

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
		m.ask(noAction, "Change the sort order",
			"Sorting is not wired up yet — this is where it will ask.")
		return true, m, nil
	}

	return false, m, nil
}

func (m Model) startFiltering() (tea.Model, tea.Cmd) {
	switch m.focus {
	case focusSidebar:
		return m, m.sidebar.StartFiltering()
	case focusWorkspace:
		if m.chatOpen {
			return m, nil
		}
		return m, m.workspace.StartFiltering()
	default:
		return m, nil
	}
}

func (m Model) handleFilterKey(msg tea.KeyPressMsg) (tea.Model, tea.Cmd) {
	switch {
	case key.Matches(msg, m.keys.Cancel):

		if m.sidebar.Filtering() {
			m.sidebar.StopFiltering()
		} else {
			m.workspace.StopFiltering()
		}
		return m, nil

	case msg.Text == "" && key.Matches(msg, m.keys.Up):
		if m.sidebar.Filtering() {
			m.sidebar.MoveUp()
		} else {
			m.workspace.MoveUp()
		}
		return m, nil

	case msg.Text == "" && key.Matches(msg, m.keys.Down):
		if m.sidebar.Filtering() {
			m.sidebar.MoveDown()
		} else {
			m.workspace.MoveDown()
		}
		return m, nil
	}

	if m.sidebar.Filtering() {
		return m, m.sidebar.UpdateFilter(msg)
	}

	return m, m.workspace.UpdateFilter(msg)
}

func (m Model) handlePanelKey(msg tea.KeyPressMsg) (tea.Model, tea.Cmd) {
	switch m.focus {
	case focusSidebar:
		return m.handleSidebarKey(msg), nil
	case focusWorkspace:
		if m.chatOpen {
			return m.handleChatKey(msg)
		}
		return m.handleWorkspaceKey(msg)
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
			m.chatOpen = section == resource.Chat
			if !m.chatOpen {
				m.workspace.Open(section)
			}
			m.focus = focusWorkspace
		}

	case key.Matches(msg, m.keys.OpenPanel):
		if section, ok := m.sidebar.Selected(); ok {
			m.chatOpen = section == resource.Chat
			if !m.chatOpen {
				m.workspace.OpenTab(section)
			}
			m.focus = focusWorkspace
		}
	}

	return m
}

func (m Model) handleChatKey(msg tea.KeyPressMsg) (tea.Model, tea.Cmd) {
	switch {
	case key.Matches(msg, m.keys.Up):
		m.chat.MoveUp()
	case key.Matches(msg, m.keys.Down):
		m.chat.MoveDown()
	case key.Matches(msg, m.keys.PageUp):
		m.chat.ScrollUp()
	case key.Matches(msg, m.keys.PageDown):
		m.chat.ScrollDown()
	case key.Matches(msg, m.keys.Top):
		m.chat.ScrollOldest()
	case key.Matches(msg, m.keys.Bottom):
		m.chat.ScrollNewest()
	case key.Matches(msg, m.keys.Compose):
		return m, m.chat.StartComposing()
	}

	return m, nil
}

func (m Model) handleComposeKey(msg tea.KeyPressMsg) (tea.Model, tea.Cmd) {
	switch {
	case key.Matches(msg, m.keys.Cancel):
		m.chat.StopComposing()
		return m, nil

	case key.Matches(msg, m.keys.Send):
		m.chat.Send()
		return m, nil

	case msg.Code == tea.KeyPgUp:
		m.chat.ScrollUp()
		return m, nil

	case msg.Code == tea.KeyPgDown:
		m.chat.ScrollDown()
		return m, nil
	}

	return m, m.chat.UpdateComposer(msg)
}

func (m Model) handleWorkspaceKey(msg tea.KeyPressMsg) (tea.Model, tea.Cmd) {
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
		return m, live.Probe(m.client, false)
	}

	return m, nil
}

func (m Model) handleTaskbarKey(msg tea.KeyPressMsg) tea.Model {
	switch {
	case key.Matches(msg, m.keys.Up):
		m.taskbar.MoveUp()

	case key.Matches(msg, m.keys.Down):
		m.taskbar.MoveDown()

	case key.Matches(msg, m.keys.Cancel):
		if task, ok := m.taskbar.Selected(); ok {
			m.ask(noAction, "Cancel "+task.Name+"?",
				"Cancelling is not wired up yet — this is where it will ask.")
		}
	}

	return m
}

func (m Model) handleModalKey(msg tea.KeyPressMsg) (tea.Model, tea.Cmd) {
	if key.Matches(msg, m.keys.Cancel) {
		return m.closeModal(), nil
	}

	switch m.modal {
	case modal.Help:
		return m.handleHelpKey(msg)

	case modal.Prompt:
		if key.Matches(msg, m.keys.Open) {
			return m.runCommand(m.prompt.Value())
		}
		return m, m.prompt.Update(msg)

	case modal.Login:
		switch {
		case key.Matches(msg, m.keys.NextPanel), key.Matches(msg, m.keys.PrevPanel):
			return m, m.login.Toggle()
		case key.Matches(msg, m.keys.Open):
			user, password, ok, cmd := m.login.Submit()
			if !ok {
				return m, cmd
			}
			return m, live.Login(m.client, user, password)
		}
		return m, m.login.Update(msg)

	case modal.Confirm:
		switch {
		case key.Matches(msg, m.keys.NextPanel), key.Matches(msg, m.keys.PrevPanel):
			m.confirm.Toggle()
		case key.Matches(msg, m.keys.Open):
			return m.answer()
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

// runCommand runs what was typed at the command prompt.
func (m Model) runCommand(line string) (tea.Model, tea.Cmd) {
	m = m.closeModal()

	word, _, _ := strings.Cut(strings.TrimSpace(line), " ")
	switch word {
	case "login":
		return m.openLogin("")

	case "logout":
		if !m.live.Account.SignedIn() {
			return m, nil
		}
		_ = m.store.Forget(m.client.URL())
		m.saved = nil
		return m, live.Logout(m.client)
	}

	return m, nil
}

func (m *Model) ask(a action, title, body string) {
	m.confirm.Ask(title, body)
	m.pending = a
	m.modal = modal.Confirm
}

func (m Model) answer() (tea.Model, tea.Cmd) {
	accepted, pending := m.confirm.Accepted(), m.pending
	m = m.closeModal()

	if accepted && pending == quitAction {
		return m.quit()
	}

	return m, nil
}

func (m Model) quit() (tea.Model, tea.Cmd) {
	m.quitting = true
	return m, tea.Quit
}

func (m Model) closeModal() Model {
	m.help.Blur()
	m.prompt.Blur()
	m.login.Blur()
	m.modal = modal.None
	m.pending = noAction

	return m
}
