package app

import (
	tea "charm.land/bubbletea/v2"

	"github.com/HilthonTT/phantom/cli/internal/client"
	"github.com/HilthonTT/phantom/cli/internal/session"
	"github.com/HilthonTT/phantom/cli/internal/tui/chat"
	"github.com/HilthonTT/phantom/cli/internal/tui/connection"
	"github.com/HilthonTT/phantom/cli/internal/tui/inspector"
	"github.com/HilthonTT/phantom/cli/internal/tui/keymap"
	"github.com/HilthonTT/phantom/cli/internal/tui/live"
	"github.com/HilthonTT/phantom/cli/internal/tui/modal"
	"github.com/HilthonTT/phantom/cli/internal/tui/resource"
	"github.com/HilthonTT/phantom/cli/internal/tui/sidebar"
	"github.com/HilthonTT/phantom/cli/internal/tui/summary"
	"github.com/HilthonTT/phantom/cli/internal/tui/taskbar"
	"github.com/HilthonTT/phantom/cli/internal/tui/theme"
	"github.com/HilthonTT/phantom/cli/internal/tui/workspace"
)

type focus int

const (
	focusSidebar focus = iota
	focusWorkspace
	focusTasks

	focusCount
)

type action int

const (
	noAction action = iota
	quitAction
	leaveAction
	actAction
)

type Model struct {
	theme  theme.Theme
	glyphs theme.Glyphs
	keys   keymap.KeyMap

	client *client.Client
	store  session.Store
	live   live.State

	// saved is the session found on disk at startup, checked by Init.
	saved *client.Session

	// loginOffered is set once the login form has opened by itself, so a
	// dismissed form is not forced open again on the next probe.
	loginOffered bool

	sync chatLive

	sidebar    sidebar.Model
	workspace  workspace.Model
	inspector  inspector.Model
	taskbar    taskbar.Model
	summary    summary.Model
	connection connection.Model
	chat       chat.Model

	chatOpen bool

	help    modal.HelpModel
	prompt  modal.PromptModel
	confirm modal.ConfirmModel
	login   modal.LoginModel
	menu    modal.MenuModel
	input   modal.InputModel
	notice  modal.NoticeModel

	// actions are the menu's choices, and acting the action being asked
	// about or confirmed.
	actions []live.Action
	acting  live.Action
	modal   modal.Kind
	pending action

	focus focus

	width  int
	height int

	quitting bool
}

// New builds the console for the server c talks to, resuming the session
// store holds for it, if any.
func New(c *client.Client, store session.Store) Model {
	t := theme.Default()
	g := theme.UnicodeGlyphs()
	keys := keymap.Default()
	state := live.New(c)

	ws := workspace.New(t, g, resource.Overview)
	ws.SetSource(state.Listing)

	var saved *client.Session
	if s, ok, err := store.Load(c.URL()); err == nil && ok {
		saved = &s
	}

	return Model{
		theme:  t,
		glyphs: g,
		keys:   keys,

		client: c,
		store:  store,
		live:   state,
		saved:  saved,

		sidebar:    sidebar.New(t, g),
		workspace:  ws,
		inspector:  inspector.New(t, g),
		taskbar:    taskbar.New(t, g),
		summary:    summary.New(t),
		connection: connection.New(t, g, state.Server()),
		chat:       chat.New(t, g),

		help:    modal.NewHelp(t, g, keys),
		prompt:  modal.NewPrompt(t),
		confirm: modal.NewConfirm(t),
		login:   modal.NewLogin(t),
		menu:    modal.NewMenu(t, g),
		input:   modal.NewInput(t),
		notice:  modal.NewNotice(t),

		focus: focusWorkspace,
	}
}

func (m Model) Init() tea.Cmd {
	if m.saved == nil {
		return live.Probe(m.client, true)
	}

	return tea.Batch(live.Probe(m.client, true), live.Resume(m.client, *m.saved))
}

func (m Model) openSection() resource.Section {
	if m.chatOpen {
		return resource.Chat
	}

	return m.workspace.Section()
}

func (m Model) filtering() bool {
	return m.sidebar.Filtering() || m.workspace.Filtering()
}
