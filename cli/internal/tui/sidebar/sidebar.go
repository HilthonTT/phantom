package sidebar

import (
	"strings"

	"charm.land/bubbles/v2/textinput"
	tea "charm.land/bubbletea/v2"

	"github.com/HilthonTT/phantom/cli/internal/tui/resource"
	"github.com/HilthonTT/phantom/cli/internal/tui/theme"
)

const Width = 24

type Model struct {
	theme  theme.Theme
	glyphs theme.Glyphs

	height int

	cursor int

	filter    textinput.Model
	filtering bool
}

func New(t theme.Theme, g theme.Glyphs) Model {
	filter := t.Input(" / ", "filter sections", t.Palette.Surface)
	filter.SetWidth(Width - 8)

	return Model{theme: t, glyphs: g, filter: filter}
}

func (m *Model) SetHeight(h int) { m.height = h }

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

func (m Model) Selected() (resource.Section, bool) {
	sections := m.Sections()
	if len(sections) == 0 {
		return 0, false
	}

	return sections[min(m.cursor, len(sections)-1)], true
}

func (m Model) Filtering() bool { return m.filtering }

func (m *Model) MoveUp()   { m.cursor = max(m.cursor-1, 0) }
func (m *Model) MoveDown() { m.cursor = min(m.cursor+1, max(len(m.Sections())-1, 0)) }

func (m *Model) StartFiltering() tea.Cmd {
	m.filtering = true
	return m.filter.Focus()
}

func (m *Model) StopFiltering() {
	m.filtering = false
	m.filter.Blur()
	m.filter.SetValue("")
	m.cursor = 0
}

func (m *Model) UpdateFilter(msg tea.Msg) tea.Cmd {
	var cmd tea.Cmd
	m.filter, cmd = m.filter.Update(msg)
	m.cursor = min(m.cursor, max(len(m.Sections())-1, 0))

	return cmd
}
