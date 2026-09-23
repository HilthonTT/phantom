package summary

import (
	"strconv"

	"github.com/HilthonTT/phantom/cli/internal/tui/detail"
	"github.com/HilthonTT/phantom/cli/internal/tui/panel"
	"github.com/HilthonTT/phantom/cli/internal/tui/resource"
	"github.com/HilthonTT/phantom/cli/internal/tui/theme"
)

type Model struct {
	theme theme.Theme

	width  int
	height int
}

func New(t theme.Theme) Model { return Model{theme: t} }

func (m *Model) SetSize(width, height int) { m.width, m.height = width, height }

func (m Model) Render(row resource.Row, ok bool, focused bool) string {
	p := panel.New(m.theme.PanelConfig(m.width, m.height, focused))
	p.SetTitle("Summary")

	if !ok || len(row.Detail) == 0 {
		p.AddLine("")
		p.AddLine(m.theme.Faint.Render(detail.Indent + "nothing selected"))
		return p.Render()
	}

	p.AddLine("")

	if hidden := detail.Fill(p, m.theme, row.Detail, labelWidth); hidden > 0 {
		p.SetInfo("+" + strconv.Itoa(hidden) + " more")
	}

	return p.Render()
}

const labelWidth = 12
