package sidebar

import (
	"fmt"
	"strings"

	"github.com/HilthonTT/phantom/cli/internal/tui/panel"
	"github.com/HilthonTT/phantom/cli/internal/tui/resource"
)

// Render draws the sidebar. `open` is the section the workspace is currently
// showing, which is marked so that the cursor and the open section can be told
// apart when they have been moved off each other.
func (m Model) Render(focused bool, open resource.Section) string {
	p := panel.New(m.theme.PanelConfig(Width, m.height, focused))
	p.SetTitle("phantom")

	p.AddLine("")
	p.AddLine(m.filter.View())
	p.AddLine("")

	sections := m.Sections()
	if len(sections) == 0 {
		p.AddLine(m.theme.Faint.Render("  nothing matches"))
		return p.Render()
	}

	m.renderSections(p, sections, focused, open)
	p.SetInfo(fmt.Sprintf("%d/%d", m.cursor+1, len(sections)))

	return p.Render()
}

func (m Model) renderSections(p *panel.Panel, sections []resource.Section, focused bool, open resource.Section) {
	heading := resource.Group(-1)

	for i, section := range sections {
		if p.Remaining() < 1 {
			return
		}

		if group := section.Group(); group != heading {
			heading = group
			if i > 0 {
				p.AddLine("")
			}
			p.AddLine(m.heading(group, p.ContentWidth()))
		}

		p.AddLine(m.entry(section, i == m.cursor && focused && !m.filtering, section == open))
	}
}

// heading is a group title with a rule either side of it, so the groups read
// as separators rather than as entries.
func (m Model) heading(g resource.Group, width int) string {
	const lead = 2

	label := " " + g.String() + " "
	rule := max(width-panel.Width(label)-lead-2, 0)

	return m.theme.Faint.Render(" "+strings.Repeat(m.glyphs.Divider, lead)) +
		m.theme.Heading.Render(label) +
		m.theme.Faint.Render(strings.Repeat(m.glyphs.Divider, rule))
}

// entry is one section: a cursor column, the section's glyph, and its name.
func (m Model) entry(s resource.Section, underCursor, open bool) string {
	cursor := "  "
	if underCursor {
		cursor = " " + m.glyphs.Cursor
	}

	style := m.theme.Text
	if open {
		style = m.theme.RowSelected
	}

	return m.theme.Cursor.Render(cursor) +
		style.Render(" "+m.glyph(s)+" "+s.String())
}

func (m Model) glyph(s resource.Section) string {
	switch s {
	case resource.Overview:
		return m.glyphs.Server
	case resource.Rooms:
		return m.glyphs.Room
	case resource.Users:
		return m.glyphs.User
	case resource.Federation:
		return m.glyphs.Federated
	case resource.Media:
		return m.glyphs.Media
	case resource.Tasks:
		return m.glyphs.Task
	case resource.Logs:
		return m.glyphs.Log
	default:
		return m.glyphs.Config
	}
}
