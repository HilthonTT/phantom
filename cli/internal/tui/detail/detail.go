// Package detail renders the labelled lines that the inspector, the summary
// box and the connection box are all built from.
//
// A detail line is a label in the muted colour, padded to a fixed column, and
// a value tinted by its state. The label column is fixed rather than measured
// so that values do not shift sideways as the cursor moves between rows whose
// labels happen to be different lengths.
package detail

import (
	"github.com/HilthonTT/phantom/cli/internal/tui/panel"
	"github.com/HilthonTT/phantom/cli/internal/tui/resource"
	"github.com/HilthonTT/phantom/cli/internal/tui/theme"
)

// Indent is the left margin every detail line carries.
const Indent = "  "

// Line renders one field into the given width, with the label occupying
// labelWidth columns.
//
// The value is truncated from the right. These are mostly Matrix identifiers,
// and it is their head that tells them apart — `#random` and `#general` share
// the `:phantom.chat` that a tail-preserving truncation would keep.
func Line(t theme.Theme, f resource.Field, labelWidth, width int) string {
	label := panel.Pad(panel.Truncate(f.Label, labelWidth), labelWidth)
	room := max(width-labelWidth-panel.Width(Indent)-1, 1)

	return t.Muted.Render(Indent+label) +
		t.ForState(f.Emphasis).Render(panel.Truncate(f.Value, room))
}

// Fill adds as many of the fields to the panel as it has room for, and returns
// how many were left over.
func Fill(p *panel.Panel, t theme.Theme, fields []resource.Field, labelWidth int) int {
	for i, f := range fields {
		if p.Remaining() == 0 {
			return len(fields) - i
		}
		p.AddLine(Line(t, f, labelWidth, p.ContentWidth()))
	}

	return 0
}
