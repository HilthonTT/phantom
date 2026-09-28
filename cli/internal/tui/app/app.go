package app

import (
	tea "charm.land/bubbletea/v2"

	"github.com/HilthonTT/phantom/cli/internal/tui/chat"
	"github.com/HilthonTT/phantom/cli/internal/tui/connection"
	"github.com/HilthonTT/phantom/cli/internal/tui/inspector"
	"github.com/HilthonTT/phantom/cli/internal/tui/keymap"
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
)

type Model struct {
	theme  theme.Theme
	glyphs theme.Glyphs
	keys   keymap.KeyMap

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
	modal   modal.Kind
	pending action

	focus focus

	width  int
	height int

	quitting bool
}

func New() Model {
	t := theme.Default()
	g := theme.UnicodeGlyphs()
	keys := keymap.Default()

	return Model{
		theme:  t,
		glyphs: g,
		keys:   keys,

		sidebar:    sidebar.New(t, g),
		workspace:  workspace.New(t, g, resource.Overview),
		inspector:  inspector.New(t, g),
		taskbar:    taskbar.New(t, g),
		summary:    summary.New(t),
		connection: connection.New(t, g),
		chat:       chat.New(t, g),

		help:    modal.NewHelp(t, g, keys),
		prompt:  modal.NewPrompt(t),
		confirm: modal.NewConfirm(t),

		focus: focusWorkspace,
	}
}

func (m Model) Init() tea.Cmd { return nil }

func (m Model) openSection() resource.Section {
	if m.chatOpen {
		return resource.Chat
	}

	return m.workspace.Section()
}

func (m Model) filtering() bool {
	return m.sidebar.Filtering() || m.workspace.Filtering()
}
