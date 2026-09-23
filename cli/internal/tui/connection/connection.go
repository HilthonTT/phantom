package connection

import (
	"github.com/HilthonTT/phantom/cli/internal/tui/detail"
	"github.com/HilthonTT/phantom/cli/internal/tui/panel"
	"github.com/HilthonTT/phantom/cli/internal/tui/resource"
	"github.com/HilthonTT/phantom/cli/internal/tui/sample"
	"github.com/HilthonTT/phantom/cli/internal/tui/theme"
)

type Model struct {
	theme  theme.Theme
	glyphs theme.Glyphs

	server resource.Server

	width  int
	height int
}

func New(t theme.Theme, g theme.Glyphs) Model {
	return Model{theme: t, glyphs: g, server: sample.Server()}
}

func (m *Model) SetSize(width, height int) { m.width, m.height = width, height }

func (m Model) Render() string {
	p := panel.New(m.theme.PanelConfig(m.width, m.height, false))
	p.SetTitle("Connection")

	p.AddLine("")
	p.AddLine(m.theme.Title.Render(detail.Indent + panel.Truncate(m.server.Name, p.ContentWidth()-2)))
	p.AddLine(m.status(p.ContentWidth()))

	detail.Fill(p, m.theme, m.server.Facts, labelWidth)

	p.SetInfo(m.server.Admin)

	return p.Render()
}

func (m Model) status(width int) string {
	state := m.theme.ForState(m.server.State)
	dot := state.Render(detail.Indent + m.glyphs.Marked + " ")
	status := state.Render(m.server.Status)
	version := m.theme.Faint.Render("  " + m.server.Version)

	return panel.Truncate(dot+status+version, width)
}

const labelWidth = 12
