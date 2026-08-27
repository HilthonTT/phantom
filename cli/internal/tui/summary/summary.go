// Package summary draws the middle box of the footer: a few facts about
// whatever the workspace cursor is on.
//
// It overlaps the inspector deliberately. The inspector is the full record and
// is only drawn on a wide terminal; the summary is the two or three fields
// worth glancing at, and is always there.
package summary

import (
	"strconv"

	"github.com/HilthonTT/phantom/cli/internal/tui/detail"
	"github.com/HilthonTT/phantom/cli/internal/tui/panel"
	"github.com/HilthonTT/phantom/cli/internal/tui/resource"
	"github.com/HilthonTT/phantom/cli/internal/tui/theme"
)

// Model is the summary box's size. What it shows is passed to [Model.Render].
type Model struct {
	theme theme.Theme

	width  int
	height int
}

// New returns a summary box.
func New(t theme.Theme) Model { return Model{theme: t} }

// SetSize sets the box's extent, borders included.
func (m *Model) SetSize(width, height int) { m.width, m.height = width, height }

// Render draws the fields of the given row, as many as fit.
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

// labelWidth is the column the values line up on.
const labelWidth = 12
