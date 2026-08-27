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

// The help menu's preferred extent, borders included.
const (
	helpWidth  = 68
	helpHeight = 26
)

// keyColumn is the width the hotkeys are right-aligned in, so the descriptions
// start on one column however long the keys are.
const keyColumn = 18

// HelpModel is the searchable list of every binding.
type HelpModel struct {
	theme  theme.Theme
	glyphs theme.Glyphs

	entries []keymap.Entry
	cursor  int
	top     int

	search textinput.Model
}

// NewHelp returns a help menu over the given binding set.
func NewHelp(t theme.Theme, g theme.Glyphs, k keymap.KeyMap) HelpModel {
	search := t.Input(" / ", "search the hotkeys", t.Palette.Raised)
	search.SetWidth(helpWidth - 10)

	return HelpModel{theme: t, glyphs: g, entries: k.Entries(), search: search}
}

// Focus gives the search box the keyboard, which it holds for as long as the
// help menu is open.
func (m *HelpModel) Focus() tea.Cmd {
	m.cursor, m.top = 0, 0
	m.search.SetValue("")

	return m.search.Focus()
}

// Blur takes the keyboard back.
func (m *HelpModel) Blur() { m.search.Blur() }

// Update feeds a message to the search box and re-clamps the cursor.
func (m *HelpModel) Update(msg tea.Msg) tea.Cmd {
	var cmd tea.Cmd
	m.search, cmd = m.search.Update(msg)
	m.moveTo(m.cursor)

	return cmd
}

// MoveUp and MoveDown walk the cursor over the matching bindings, skipping the
// headings between them.
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

	visible := helpHeight - 5
	if m.cursor < m.top {
		m.top = m.cursor
	}
	if m.cursor >= m.top+visible {
		m.top = m.cursor - visible + 1
	}
	m.top = max(m.top, 0)
}

// matching is the entries that survive the search, with any heading that would
// be left with nothing under it dropped.
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

// Render draws the help menu at the size the terminal allows.
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

// rank is the cursor's position counted in bindings, ignoring headings, which
// is the number worth printing in the border.
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

// entry is one line: a heading, or a right-aligned hotkey and what it does.
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
