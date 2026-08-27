// Package inspector draws the detail panel down the right of the interface.
//
// It shows the row the workspace's cursor is on, one labelled field per line.
// It has no cursor of its own — it follows the workspace, the way superfile's
// preview pane follows the file panel.
package inspector

import (
	"strconv"

	"github.com/HilthonTT/phantom/cli/internal/tui/theme"
)

// Width is how wide the inspector is drawn, borders included.
const Width = 36

// MinLayoutWidth is the narrowest terminal the inspector is drawn in at all.
// Below it the workspace needs every column there is.
const MinLayoutWidth = 110

// Model is the inspector's state, which is only its size — the row it shows is
// passed to [Model.Render] by whoever owns the workspace.
type Model struct {
	theme  theme.Theme
	glyphs theme.Glyphs

	height int
}

// New returns an inspector.
func New(t theme.Theme, g theme.Glyphs) Model {
	return Model{theme: t, glyphs: g}
}

// SetHeight sets how many lines the inspector occupies, borders included.
func (m *Model) SetHeight(h int) { m.height = h }

// labelWidth is the column the values line up on.
const labelWidth = 13

func itoa(n int) string { return strconv.Itoa(n) }
