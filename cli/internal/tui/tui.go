package tui

import (
	tea "charm.land/bubbletea/v2"

	"github.com/HilthonTT/phantom/cli/internal/client"
	"github.com/HilthonTT/phantom/cli/internal/tui/app"
)

func Run(c *client.Client) error {
	_, err := tea.NewProgram(app.New(c)).Run()

	return err
}
