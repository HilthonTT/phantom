package inspector

import (
	"strconv"

	"github.com/HilthonTT/phantom/cli/internal/tui/theme"
)

const Width = 36

const MinLayoutWidth = 110

type Model struct {
	theme  theme.Theme
	glyphs theme.Glyphs

	height int
}

func New(t theme.Theme, g theme.Glyphs) Model {
	return Model{theme: t, glyphs: g}
}

func (m *Model) SetHeight(h int) { m.height = h }

const labelWidth = 13

func itoa(n int) string { return strconv.Itoa(n) }
