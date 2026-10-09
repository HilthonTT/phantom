package modal

import (
	"github.com/HilthonTT/phantom/cli/internal/tui/panel"
	"github.com/HilthonTT/phantom/cli/internal/tui/theme"
)

const menuWidth = 46

// MenuModel offers a few choices about one thing, such as the actions on a
// table row.
type MenuModel struct {
	theme  theme.Theme
	glyphs theme.Glyphs

	title   string
	choices []string
	cursor  int
}

func NewMenu(t theme.Theme, g theme.Glyphs) MenuModel {
	return MenuModel{theme: t, glyphs: g}
}

func (m *MenuModel) Open(title string, choices []string) {
	m.title, m.choices, m.cursor = title, choices, 0
}

func (m *MenuModel) MoveUp()   { m.cursor = max(m.cursor-1, 0) }
func (m *MenuModel) MoveDown() { m.cursor = min(m.cursor+1, len(m.choices)-1) }

// Chosen is the index of the choice under the cursor.
func (m MenuModel) Chosen() int { return m.cursor }

func (m MenuModel) Render(width, height int) string {
	w, h := size(menuWidth, len(m.choices)+5, width, height)

	p := panel.New(m.theme.ModalConfig(w, h))
	p.SetTitle(panel.Truncate(m.title, max(w-8, 1)))

	p.AddLine("")
	for i, choice := range m.choices {
		if p.Remaining() < 1 {
			break
		}

		line := "    " + choice
		style := m.theme.ModalHint
		if i == m.cursor {
			line = "  " + m.glyphs.Cursor + " " + choice
			style = m.theme.ModalTitle
		}
		p.AddLine(style.Render(panel.Pad(panel.Truncate(line, p.ContentWidth()), p.ContentWidth())))
	}

	p.SetInfo("enter chooses · esc closes")

	return p.Render()
}
