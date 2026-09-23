package tui

import (
	tea "charm.land/bubbletea/v2"

	"github.com/HilthonTT/phantom/cli/internal/tui/app"
)

func Run() error {
	_, err := tea.NewProgram(app.New()).Run()

	return err
}
