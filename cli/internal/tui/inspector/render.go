package inspector

import (
	"github.com/HilthonTT/phantom/cli/internal/tui/detail"
	"github.com/HilthonTT/phantom/cli/internal/tui/panel"
	"github.com/HilthonTT/phantom/cli/internal/tui/resource"
)

// Render draws the inspector for the given row. `section` names what the row
// came from, and is printed in the border.
func (m Model) Render(section resource.Section, row resource.Row, ok bool) string {
	p := panel.New(m.theme.PanelConfig(Width, m.height, false))
	p.SetTitle("Inspector")

	if !ok || len(row.Detail) == 0 {
		p.AddLine("")
		p.AddLine(m.theme.Faint.Render(detail.Indent + " nothing selected"))
		return p.Render()
	}

	p.AddLine("")
	p.AddLine(m.heading(row.Detail[0], p.ContentWidth()))
	p.AddDivider()

	hidden := detail.Fill(p, m.theme, row.Detail[1:], labelWidth)

	p.SetInfo(section.String())
	if hidden > 0 {
		p.SetInfo(section.String(), "+"+itoa(hidden))
	}

	return p.Render()
}

// heading is the row's first field, printed on its own as the title of what is
// being inspected.
func (m Model) heading(f resource.Field, width int) string {
	return m.theme.Cursor.Render(detail.Indent+m.glyphs.Bullet+" ") +
		m.theme.Title.Render(panel.Truncate(f.Value, max(width-4, 1)))
}
