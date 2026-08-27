// Package tui is the phantom admin console: a terminal interface for looking
// at a running homeserver.
//
// The layout is a section navigator down the left, one or more resource
// listings across the middle, a detail panel on the right, and a row of boxes
// along the bottom for running tasks, the current selection and the connection
// itself. It is modelled on superfile, whose panel-and-footer arrangement
// suits an admin console for the same reason it suits a file manager: several
// listings worth reading against each other, with the state of the session
// always in view underneath them.
//
// Nothing in here talks to a homeserver yet. The interface is complete and the
// data behind it is not.
package tui

import (
	tea "charm.land/bubbletea/v2"

	"github.com/HilthonTT/phantom/cli/internal/tui/app"
)

// Run starts the console and blocks until it is quit.
func Run() error {
	_, err := tea.NewProgram(app.New()).Run()

	return err
}
