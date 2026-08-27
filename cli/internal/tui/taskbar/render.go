package taskbar

import (
	"fmt"
	"strings"

	"github.com/HilthonTT/phantom/cli/internal/tui/panel"
	"github.com/HilthonTT/phantom/cli/internal/tui/resource"
)

// Render draws the task bar.
func (m Model) Render(focused bool) string {
	p := panel.New(m.theme.PanelConfig(m.width, m.height, focused))
	p.SetTitle("Tasks")

	if len(m.tasks) == 0 {
		p.AddLine("")
		p.AddLine(m.theme.Faint.Render("  nothing running"))
		return p.Render()
	}

	for i := m.top; i < len(m.tasks) && p.Remaining() >= 2; i++ {
		m.renderTask(p, m.tasks[i], i == m.cursor && focused)
	}

	p.SetInfo(fmt.Sprintf("%d/%d", m.cursor+1, len(m.tasks)))

	return p.Render()
}

// renderTask draws one task: a name line carrying its state glyph, then its
// progress bar, both behind the same cursor rail.
func (m Model) renderTask(p *panel.Panel, t resource.Task, underCursor bool) {
	rail := m.theme.Faint.Render("  ")
	if underCursor {
		rail = m.theme.Cursor.Render("┃ ")
	}

	glyph := m.theme.ForState(t.State).Render(" " + m.glyphs.GlyphForState(t.State))

	// The name shares its line with the state glyph on the right.
	name := panel.Truncate(t.Name, p.ContentWidth()-panel.Width(rail)-2)
	gap := max(p.ContentWidth()-panel.Width(rail)-panel.Width(name)-2, 0)

	p.AddLine(rail + m.theme.Text.Render(name+strings.Repeat(" ", gap)) + glyph)

	if p.Remaining() == 0 {
		return
	}

	bar := m.bars[t.State]
	bar.SetWidth(max(p.ContentWidth()-panel.Width(rail)-8, 4))

	p.AddLine(rail + bar.ViewAs(t.Progress) +
		m.theme.Muted.Render(fmt.Sprintf(" %3.0f%%", t.Progress*100)))

	if p.Remaining() > 0 {
		p.AddLine(rail + m.theme.Faint.Render(panel.Truncate(t.Note, p.ContentWidth()-panel.Width(rail))))
	}
}
