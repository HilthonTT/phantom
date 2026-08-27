package theme

import (
	"charm.land/lipgloss/v2"

	"github.com/HilthonTT/phantom/cli/internal/tui/resource"
)

// ForState is the style a value in the given state is drawn in, on a panel's
// background.
//
// Every part of the interface that tints by state — a listing row, a task, a
// federation peer, the connection dot — asks here, so the same state is the
// same colour wherever it appears.
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

// GlyphForState is the symbol that goes with a state.
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
