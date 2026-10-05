package tui

import (
	tea "charm.land/bubbletea/v2"

	"github.com/HilthonTT/phantom/cli/internal/client"
	"github.com/HilthonTT/phantom/cli/internal/session"
	"github.com/HilthonTT/phantom/cli/internal/tui/app"
)

func Run(c *client.Client, store session.Store) error {
	_, err := tea.NewProgram(app.New(c, store)).Run()

	return err
}
