package sidebar

import (
	"fmt"
	"strings"

	"github.com/HilthonTT/phantom/cli/internal/tui/panel"
	"github.com/HilthonTT/phantom/cli/internal/tui/resource"
)

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

	var lines []string
	cursorLine := 0
	for i, section := range sections {
		if group := section.Group(); group != heading {
			heading = group
			if i > 0 {
				lines = append(lines, "")
			}
			lines = append(lines, m.heading(group, p.ContentWidth()))
		}

		if i == m.cursor {
			cursorLine = len(lines)
		}
		lines = append(lines, m.entry(section, i == m.cursor && focused && !m.filtering, section == open))
	}

	// More sections than rows: slide the window just far enough that the
	// cursor stays on screen.
	height := p.Remaining()
	start := min(max(cursorLine-height+1, 0), max(len(lines)-height, 0))
	p.AddLines(lines[start:min(start+height, len(lines))]...)
}

func (m Model) heading(g resource.Group, width int) string {
	const lead = 2

	label := " " + g.String() + " "
	rule := max(width-panel.Width(label)-lead-2, 0)

	return m.theme.Faint.Render(" "+strings.Repeat(m.glyphs.Divider, lead)) +
		m.theme.Heading.Render(label) +
		m.theme.Faint.Render(strings.Repeat(m.glyphs.Divider, rule))
}

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
	case resource.Services:
		return m.glyphs.Service
	case resource.API:
		return m.glyphs.API
	case resource.Rooms:
		return m.glyphs.Room
	case resource.Users:
		return m.glyphs.User
	case resource.Devices:
		return m.glyphs.Device
	case resource.Tokens:
		return m.glyphs.Token
	case resource.Federation:
		return m.glyphs.Federated
	case resource.Appservices:
		return m.glyphs.Bridge
	case resource.Media:
		return m.glyphs.Media
	case resource.Tasks:
		return m.glyphs.Task
	case resource.Reports:
		return m.glyphs.Report
	case resource.Logs:
		return m.glyphs.Log
	case resource.Chat:
		return m.glyphs.Chat
	default:
		return m.glyphs.Config
	}
}
