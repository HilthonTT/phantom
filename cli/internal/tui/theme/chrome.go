package theme

import "github.com/HilthonTT/phantom/cli/internal/tui/panel"

// PanelConfig is the chrome for a panel in the main layout, coloured for
// whether it currently has the keyboard.
//
// Every component asks for its box here rather than assembling one, so the
// borders of the layout cannot drift apart from each other.
func (t Theme) PanelConfig(width, height int, focused bool) panel.Config {
	border := t.Palette.Border
	if focused {
		border = t.Palette.BorderActive
	}

	return panel.Config{
		Width:     width,
		Height:    height,
		Border:    Border(),
		BorderFG:  border,
		BorderBG:  t.Palette.Surface,
		ContentFG: t.Palette.Text,
		ContentBG: t.Palette.Surface,
	}
}

// ModalConfig is the chrome for a box drawn over the top of the layout. A
// modal always has the keyboard, so its border is always the active one.
func (t Theme) ModalConfig(width, height int) panel.Config {
	return panel.Config{
		Width:     width,
		Height:    height,
		Border:    Border(),
		BorderFG:  t.Palette.BorderActive,
		BorderBG:  t.Palette.Raised,
		ContentFG: t.Palette.Text,
		ContentBG: t.Palette.Raised,
	}
}
