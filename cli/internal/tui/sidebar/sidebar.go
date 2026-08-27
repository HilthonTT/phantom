// Package sidebar draws the section navigator down the left of the interface.
//
// It lists every section of the admin console under its heading, filtered by
// what has been typed into the box at the top of it. Which section the cursor
// is on is the sidebar's own state; which section is *open* belongs to the
// workspace, and is passed in so the open one can be marked.
package sidebar

import (
	"strings"

	"charm.land/bubbles/v2/textinput"
	tea "charm.land/bubbletea/v2"

	"github.com/HilthonTT/phantom/cli/internal/tui/resource"
	"github.com/HilthonTT/phantom/cli/internal/tui/theme"
)

// Width is how wide the sidebar is drawn, borders included. It is fixed: the
// section names are known, and a navigator that changes width as the terminal
// does is harder to aim at than one that does not.
const Width = 24

// Model is the sidebar's state.
type Model struct {
	theme  theme.Theme
	glyphs theme.Glyphs

	height int

	// cursor indexes into the filtered sections, not into every section.
	cursor int

	filter    textinput.Model
	filtering bool
}

// New returns a sidebar with the cursor on the first section.
func New(t theme.Theme, g theme.Glyphs) Model {
	filter := t.Input(" / ", "filter sections", t.Palette.Surface)
	filter.SetWidth(Width - 8)

	return Model{theme: t, glyphs: g, filter: filter}
}

// SetHeight sets how many lines the sidebar occupies, borders included.
func (m *Model) SetHeight(h int) { m.height = h }

// Sections are the sections that survive the current filter.
func (m Model) Sections() []resource.Section {
	query := strings.ToLower(strings.TrimSpace(m.filter.Value()))
	if query == "" {
		return resource.Sections()
	}

	var kept []resource.Section
	for _, s := range resource.Sections() {
		if strings.Contains(strings.ToLower(s.String()), query) {
			kept = append(kept, s)
		}
	}

	return kept
}

// Selected is the section under the cursor, and whether there is one — a
// filter that matches nothing leaves none.
func (m Model) Selected() (resource.Section, bool) {
	sections := m.Sections()
	if len(sections) == 0 {
		return 0, false
	}

	return sections[min(m.cursor, len(sections)-1)], true
}

// Filtering reports whether the filter box has the keyboard.
func (m Model) Filtering() bool { return m.filtering }

// MoveUp and MoveDown walk the cursor over the filtered sections, stopping at
// each end rather than wrapping — the list is short enough that wrapping only
// surprises.
func (m *Model) MoveUp()   { m.cursor = max(m.cursor-1, 0) }
func (m *Model) MoveDown() { m.cursor = min(m.cursor+1, max(len(m.Sections())-1, 0)) }

// StartFiltering gives the filter box the keyboard.
func (m *Model) StartFiltering() tea.Cmd {
	m.filtering = true
	return m.filter.Focus()
}

// StopFiltering takes the keyboard back and empties the box.
func (m *Model) StopFiltering() {
	m.filtering = false
	m.filter.Blur()
	m.filter.SetValue("")
	m.cursor = 0
}

// UpdateFilter feeds a message to the filter box. It is only called while
// [Model.Filtering] is true.
func (m *Model) UpdateFilter(msg tea.Msg) tea.Cmd {
	var cmd tea.Cmd
	m.filter, cmd = m.filter.Update(msg)
	m.cursor = min(m.cursor, max(len(m.Sections())-1, 0))

	return cmd
}
