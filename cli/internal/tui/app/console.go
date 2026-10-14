package app

import (
	"strings"
	"time"

	"charm.land/bubbles/v2/key"
	tea "charm.land/bubbletea/v2"

	"github.com/HilthonTT/phantom/cli/internal/tui/live"
	"github.com/HilthonTT/phantom/cli/internal/tui/modal"
	"github.com/HilthonTT/phantom/cli/internal/tui/resource"
)

// The keys that belong to one section, and the room inspector they open:
// following and filtering the log, paging the audit log, and looking inside
// a room.

// inputPurpose is what the input box's line is for.
type inputPurpose int

const (
	// inputAction is the text an action asks for: a password, a token.
	inputAction inputPurpose = iota

	// inputLogTarget and inputLogSearch narrow the log.
	inputLogTarget
	inputLogSearch

	// inputConfirmName is the name typed to confirm an action that cannot
	// be undone.
	inputConfirmName
)

// handleSectionKey takes the keys only the open section answers, reporting
// whether it took this one.
func (m Model) handleSectionKey(msg tea.KeyPressMsg) (bool, tea.Model, tea.Cmd) {
	switch m.workspace.Section() {
	case resource.Rooms:
		if key.Matches(msg, m.keys.Inspect) {
			row, ok := m.workspace.Selected()
			if !ok || len(row.Ref) == 0 {
				return true, m, nil
			}
			model, cmd := m.inspectRoom(row.Ref[0], row.Cells[0], roomVersion(row))
			return true, model, cmd
		}

	case resource.Logs:
		return m.handleLogKey(msg)

	case resource.Audit:
		if key.Matches(msg, m.keys.Older) {
			next := m.live.Admin.Audit.Next
			if next == "" || !m.live.Account.Admin {
				return true, m, nil
			}
			return true, m, live.FetchOlderAudit(m.client, m.live.Admin.Gen, next)
		}
	}

	return false, m, nil
}

func (m Model) handleLogKey(msg tea.KeyPressMsg) (bool, tea.Model, tea.Cmd) {
	logs := m.live.Admin.Logs

	switch {
	case key.Matches(msg, m.keys.LogFollow):
		logs.Follow, logs.Paused = !logs.Follow, false
		m = m.withLogs(logs)
		if !logs.Follow {
			return true, m, nil
		}
		m.workspace.Top()
		tick := m.startLogTick()
		return true, m, tea.Batch(m.fetchSection(resource.Logs), tick)

	case key.Matches(msg, m.keys.LogPause):
		if !logs.Follow {
			m.notify("Not following", "f follows the log; p then pauses and resumes it.", true)
			return true, m, nil
		}
		logs.Paused = !logs.Paused
		m = m.withLogs(logs)
		if logs.Paused {
			return true, m, nil
		}
		tick := m.startLogTick()
		return true, m, tea.Batch(m.fetchSection(resource.Logs), tick)

	case key.Matches(msg, m.keys.LogLevel):
		logs.Filter = logs.Filter.NextLevel()
		return true, m.withLogs(logs), nil

	case key.Matches(msg, m.keys.LogTarget):
		m.modal, m.inputFor = modal.Input, inputLogTarget
		return true, m, m.input.Open("Show the lines of one target", logs.Filter.Target,
			"a module, or part of one, such as sending; empty shows all", false)

	case key.Matches(msg, m.keys.LogSearch):
		m.modal, m.inputFor = modal.Input, inputLogSearch
		return true, m, m.input.Open("Search the log", logs.Filter.Text,
			"matches the message, target and span; empty shows all", false)

	case key.Matches(msg, m.keys.LogDebug):
		if !m.live.Account.Admin {
			m.notify("Admins only", "Sign in as an admin to change what the server logs.", true)
			return true, m, nil
		}
		debug := logs.Debug == nil || !*logs.Debug
		return true, m, live.SetLogCapture(m.client, m.live.Admin.Gen, debug)
	}

	return false, m, nil
}

// withLogs changes the log view and redraws the listings from it.
func (m Model) withLogs(logs live.LogView) Model {
	m.live = m.live.WithLogs(func(live.LogView) live.LogView { return logs })
	m.workspace.SetSource(m.live.Listing)

	return m
}

// startLogTick starts a fresh follow timer, retiring any running one.
func (m *Model) startLogTick() tea.Cmd {
	m.logTick++
	return live.LogTick(m.live.Admin.Gen, m.logTick)
}

// logTicked asks for the log's new lines while it is followed and on
// screen, and sets the next tick going.
func (m Model) logTicked(msg live.LogTickMsg) (tea.Model, tea.Cmd) {
	if msg.Gen != m.live.Admin.Gen || msg.Tick != m.logTick || !m.live.Admin.Logs.Polling() {
		return m, nil
	}

	next := live.LogTick(msg.Gen, msg.Tick)
	if m.chatOpen || m.roomOpen || m.workspace.Section() != resource.Logs {
		return m, next
	}

	return m, tea.Batch(m.fetchSection(resource.Logs), next)
}

func (m Model) logCaptured(msg live.LogCaptureMsg) (tea.Model, tea.Cmd) {
	if msg.Err != nil {
		m.notify("Could not change the log level", msg.Err.Error(), true)
		return m, nil
	}

	m.live = m.live.TakeLogCapture(msg)
	m.workspace.SetSource(m.live.Listing)

	body := "The log keeps info and above again."
	if msg.Debug {
		body = "The log keeps phantom's debug lines too, until set back or the server restarts."
	}
	m.notify("Log level changed", body, false)

	return m, m.fetchSection(resource.Audit)
}

// logInputGiven takes a target or text to narrow the log by.
func (m Model) logInputGiven(purpose inputPurpose, value string) Model {
	logs := m.live.Admin.Logs
	if purpose == inputLogTarget {
		logs.Filter.Target = value
	} else {
		logs.Filter.Text = value
	}

	return m.withLogs(logs)
}

// inspectRoom opens the room inspector on a room, by its ID, titled as its
// listing names it.
func (m Model) inspectRoom(roomID, title, version string) (tea.Model, tea.Cmd) {
	if !m.live.Account.Admin {
		m.notify("Admins only", "Sign in as an admin to inspect a room.", true)
		return m, nil
	}

	m.room.Open(roomID, title, version)
	m.roomOpen, m.chatOpen, m.focus = true, false, focusWorkspace

	return m, live.FetchRoom(m.client, roomID)
}

// inspectNamed opens the inspector on a room named at the prompt: by ID, or
// by a name or alias the Rooms listing shows.
func (m Model) inspectNamed(name string) (tea.Model, tea.Cmd) {
	for _, r := range m.live.Listing(resource.Rooms).Rows {
		if len(r.Ref) == 0 {
			continue
		}
		matches := r.Ref[0] == name || (len(r.Cells) > 0 && r.Cells[0] == name)
		for _, f := range r.Detail {
			if f.Label == "Alias" && f.Value == name {
				matches = true
			}
		}
		if matches {
			return m.inspectRoom(r.Ref[0], r.Cells[0], roomVersion(r))
		}
	}

	if strings.HasPrefix(name, "!") {
		return m.inspectRoom(name, name, "")
	}

	m.notify("Not found", "No room "+name+" is listed; give its ID.", true)
	return m, nil
}

func roomVersion(r resource.Row) string {
	for _, f := range r.Detail {
		if f.Label == "Version" && f.Value != "unknown" {
			return f.Value
		}
	}

	return ""
}

func (m Model) handleRoomKey(msg tea.KeyPressMsg) (tea.Model, tea.Cmd) {
	switch {
	case key.Matches(msg, m.keys.Cancel):
		m.roomOpen = false
	case key.Matches(msg, m.keys.NextView):
		m.room.NextTab()
	case key.Matches(msg, m.keys.PrevView):
		m.room.PrevTab()
	case key.Matches(msg, m.keys.Expand):
		m.room.ToggleExpand()
	case key.Matches(msg, m.keys.Up):
		m.room.MoveUp()
	case key.Matches(msg, m.keys.Down):
		m.room.MoveDown()
	case key.Matches(msg, m.keys.PageUp):
		m.room.PageUp()
	case key.Matches(msg, m.keys.PageDown):
		m.room.PageDown()
	case key.Matches(msg, m.keys.Top):
		m.room.Top()
	case key.Matches(msg, m.keys.Bottom):
		m.room.Bottom()

	case key.Matches(msg, m.keys.Refresh):
		if m.room.Loading() {
			return m, nil
		}
		m.room.StartLoading()
		return m, live.FetchRoom(m.client, m.room.RoomID)

	case key.Matches(msg, m.keys.Older):
		from := m.room.Older()
		if from == "" || m.room.Loading() {
			return m, nil
		}
		m.room.StartLoading()
		return m, live.FetchOlderEvents(m.client, m.room.RoomID, from)
	}

	return m, nil
}

// roomAnswered takes what the server sent about the room being inspected.
func (m Model) roomAnswered(msg live.RoomMsg) (tea.Model, tea.Cmd) {
	if msg.RoomID != m.room.RoomID {
		return m, nil
	}

	switch {
	case msg.Err != nil:
		m.room.Fail(msg.Err)
	case msg.Older:
		m.room.AddOlder(msg.Events, time.Now())
	default:
		m.room.Load(msg.State, msg.Events, msg.Extremities, msg.Servers, time.Now())
	}

	return m, nil
}

// confirmByName asks for the target's name to be typed before an action
// that cannot be undone.
func (m Model) confirmByName(a live.Action, name string) (tea.Model, tea.Cmd) {
	title, body := a.Confirm()
	m.acting = a
	m.modal, m.inputFor = modal.Input, inputConfirmName

	return m, m.input.Open(title, name, "type "+name+" to confirm · "+body, false)
}

// nameGiven runs the action being confirmed if what was typed names its
// target.
func (m Model) nameGiven(typed string) (tea.Model, tea.Cmd) {
	a := m.acting
	if !a.ConfirmedBy(typed) {
		m.notify("Not confirmed", "What was typed does not name "+a.TypeToConfirm()+"; nothing was done.", true)
		return m, nil
	}

	return m.act(a)
}
