package modal

import (
	"strconv"
	"strings"

	"charm.land/bubbles/v2/textinput"
	tea "charm.land/bubbletea/v2"

	"github.com/HilthonTT/phantom/cli/internal/tui/keymap"
	"github.com/HilthonTT/phantom/cli/internal/tui/panel"
	"github.com/HilthonTT/phantom/cli/internal/tui/theme"
)

const (
	helpWidth  = 68
	helpHeight = 26
)

const keyColumn = 18

type HelpModel struct {
	theme  theme.Theme
	glyphs theme.Glyphs

	entries []keymap.Entry
	cursor  int
	top     int

	width, height int

	search textinput.Model
}

func NewHelp(t theme.Theme, g theme.Glyphs, k keymap.KeyMap) HelpModel {
	search := t.Input(" / ", "search the hotkeys", t.Palette.Raised)
	search.SetWidth(helpWidth - 10)

	return HelpModel{theme: t, glyphs: g, entries: k.Entries(), search: search}
}

func (m *HelpModel) Focus() tea.Cmd {
	m.top = 0
	m.search.SetValue("")
	m.moveTo(0)

	return m.search.Focus()
}

func (m *HelpModel) SetSize(width, height int) {
	m.width, m.height = width, height
	m.moveTo(m.cursor)
}

func (m HelpModel) visibleRows() int {
	_, h := size(helpWidth, helpHeight, m.width, m.height)

	return max(h-4, 1)
}

func (m *HelpModel) Blur() { m.search.Blur() }

func (m *HelpModel) Update(msg tea.Msg) tea.Cmd {
	var cmd tea.Cmd
	m.search, cmd = m.search.Update(msg)
	m.moveTo(m.cursor)

	return cmd
}

func (m *HelpModel) MoveUp()   { m.step(-1) }
func (m *HelpModel) MoveDown() { m.step(+1) }

func (m *HelpModel) step(by int) {
	entries := m.matching()
	for i := m.cursor + by; i >= 0 && i < len(entries); i += by {
		if entries[i].Heading == "" {
			m.moveTo(i)
			return
		}
	}
}

func (m *HelpModel) moveTo(i int) {
	entries := m.matching()
	m.cursor = min(max(i, 0), max(len(entries)-1, 0))

	for m.cursor < len(entries) && entries[m.cursor].Heading != "" {
		m.cursor++
	}
	if m.cursor >= len(entries) {
		m.cursor = max(len(entries)-1, 0)
		for m.cursor > 0 && entries[m.cursor].Heading != "" {
			m.cursor--
		}
	}

	visible := m.visibleRows()
	if m.cursor < m.top {
		m.top = m.cursor
	}
	if m.cursor >= m.top+visible {
		m.top = m.cursor - visible + 1
	}
	m.top = max(m.top, 0)
}

func (m HelpModel) matching() []keymap.Entry {
	query := strings.ToLower(strings.TrimSpace(m.search.Value()))
	if query == "" {
		return m.entries
	}

	var kept []keymap.Entry
	for _, e := range m.entries {
		if e.Heading != "" {
			continue
		}
		haystack := strings.ToLower(e.Description + " " + strings.Join(e.Keys, " "))
		if strings.Contains(haystack, query) {
			kept = append(kept, e)
		}
	}

	return kept
}

func (m HelpModel) Render(width, height int) string {
	w, h := size(helpWidth, helpHeight, width, height)

	p := panel.New(m.theme.ModalConfig(w, h))
	p.SetTitle("Hotkeys")

	p.AddLine(" " + m.search.View())
	p.AddDivider()

	entries := m.matching()
	if len(entries) == 0 {
		p.AddLine("")
		p.AddLine(m.theme.ModalHint.Render("   nothing matches"))
		return p.Render()
	}

	bindings := 0
	for i := m.top; i < len(entries) && p.Remaining() > 0; i++ {
		p.AddLine(m.entry(entries[i], i == m.cursor, p.ContentWidth()))
	}
	for _, e := range entries {
		if e.Heading == "" {
			bindings++
		}
	}

	p.SetInfo(strconv.Itoa(m.rank()) + "/" + strconv.Itoa(bindings))

	return p.Render()
}

func (m HelpModel) rank() int {
	rank := 0
	for i, e := range m.matching() {
		if e.Heading != "" {
			continue
		}
		rank++
		if i == m.cursor {
			return rank
		}
	}

	return rank
}

func (m HelpModel) entry(e keymap.Entry, underCursor bool, width int) string {
	if e.Heading != "" {
		return m.theme.ModalTitle.Render(" " + e.Heading)
	}

	cursor := "  "
	if underCursor {
		cursor = " " + m.glyphs.Cursor
	}

	keys := panel.PadStart(strings.Join(e.Keys, ", "), keyColumn)
	description := panel.Truncate(e.Description, max(width-keyColumn-4, 1))

	return m.theme.Modal.Render(cursor) +
		m.theme.Hotkey.Render(keys) +
		m.theme.Modal.Render("  "+description)
}
