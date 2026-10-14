package app

import (
	tea "charm.land/bubbletea/v2"

	"github.com/HilthonTT/phantom/cli/internal/tui/live"
	"github.com/HilthonTT/phantom/cli/internal/tui/resource"
)

// resetAdmin drops the admin API's answers for whoever was signed in, and
// fetches every section afresh when the account now signed in is an admin.
func (m Model) resetAdmin() (Model, tea.Cmd) {
	var gen int
	m.live, gen = m.live.StartAdmin()
	m.workspace.SetSource(m.live.Listing)
	m.taskbar.SetTasks(nil)

	if !m.live.Account.Admin {
		return m, nil
	}

	return m, live.FetchAllAdmin(m.client, gen)
}

// refreshAdmin fetches the open section again, when the admin API serves it
// and the account may ask.
func (m Model) refreshAdmin() tea.Cmd {
	return m.fetchAdmin(m.workspace.Section())
}

func (m Model) fetchAdmin(section resource.Section) tea.Cmd {
	if m.chatOpen {
		return nil
	}

	return m.fetchSection(section)
}

// fetchSection fetches a section whether or not it is on screen, as after an
// action changes it.
func (m Model) fetchSection(section resource.Section) tea.Cmd {
	if !m.live.Account.Admin || !live.Served(section) {
		return nil
	}

	// The log asks only for the lines after the newest it holds.
	if section == resource.Logs {
		return live.FetchLogs(m.client, m.live.Admin.Gen, m.live.Admin.Logs.Cursor())
	}

	return live.FetchAdmin(m.client, m.live.Admin.Gen, section)
}

func (m Model) adminAnswered(msg live.AdminMsg) (tea.Model, tea.Cmd) {
	var ok bool
	if m.live, ok = m.live.TakeAdmin(msg); ok {
		m.workspace.SetSource(m.live.Listing)
		if msg.Section == resource.Tasks && msg.Err == nil {
			m.taskbar.SetTasks(m.live.Admin.Tasks)
		}

		// A followed log keeps the newest line under the cursor.
		if msg.Section == resource.Logs && m.live.Admin.Logs.Polling() &&
			m.workspace.Section() == resource.Logs {
			m.workspace.Top()
		}
	}

	return m, nil
}
