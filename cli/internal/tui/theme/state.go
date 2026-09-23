package theme

import (
	"charm.land/lipgloss/v2"

	"github.com/HilthonTT/phantom/cli/internal/tui/resource"
)

func (t Theme) ForState(s resource.State) lipgloss.Style {
	switch s {
	case resource.Running:
		return t.StateRunning
	case resource.Done:
		return t.StateDone
	case resource.Failed:
		return t.StateFailed
	case resource.Held:
		return t.StateHeld
	default:
		return t.Text
	}
}

func (g Glyphs) GlyphForState(s resource.State) string {
	switch s {
	case resource.Running:
		return g.Running
	case resource.Done:
		return g.Done
	case resource.Failed:
		return g.Failed
	case resource.Held:
		return g.Held
	default:
		return g.Bullet
	}
}
