package theme

import "github.com/HilthonTT/phantom/cli/internal/tui/panel"

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
