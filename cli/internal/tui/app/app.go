// Package app wires the interface together: it owns every panel, decides which
// one has the keyboard, and lays them out against the terminal it is given.
//
// Nothing here talks to a homeserver. Keys move the cursor, open and close
// panels, and open the modals; the operations those would eventually start are
// not implemented, and the listings come from
// [github.com/HilthonTT/phantom/cli/internal/tui/sample].
package app

import (
	tea "charm.land/bubbletea/v2"

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

// focus is which panel the keyboard is pointed at.
type focus int

// The panels that can hold the keyboard, in the order [keymap.KeyMap.FocusNext]
// walks them.
const (
	focusSidebar focus = iota
	focusWorkspace
	focusTasks

	focusCount
)

// Model is the whole interface.
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

	help    modal.HelpModel
	prompt  modal.PromptModel
	confirm modal.ConfirmModel
	modal   modal.Kind

	focus focus

	width  int
	height int

	quitting bool
}

// New builds the interface with the overview open.
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

		help:    modal.NewHelp(t, g, keys),
		prompt:  modal.NewPrompt(t),
		confirm: modal.NewConfirm(t),

		focus: focusWorkspace,
	}
}

// Init starts the interface. There is nothing to load, so there is no command
// to run.
func (m Model) Init() tea.Cmd { return nil }

// filtering reports whether a filter box somewhere has the keyboard, in which
// case most keys are text rather than commands.
func (m Model) filtering() bool {
	return m.sidebar.Filtering() || m.workspace.Filtering()
}
