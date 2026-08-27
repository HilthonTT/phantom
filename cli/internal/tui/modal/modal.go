// Package modal draws the boxes that open over the top of the layout: the help
// menu, the command prompt and the confirmation box.
//
// A modal does not replace the layout, it is composited over the middle of it,
// so the panels stay visible around the edges and it is obvious that the
// console is still where it was left.
package modal

import (
	"charm.land/lipgloss/v2"
)

// Kind is which modal is open, if any.
type Kind int

// The modals. None means the layout has the screen to itself.
const (
	None Kind = iota
	Help
	Prompt
	Confirm
)

// Overlay composites a modal over the middle of the layout.
//
// The layout keeps its own dimensions; the modal is placed at whole-cell
// coordinates so nothing underneath is shifted by half a column.
func Overlay(layout, box string, width, height int) string {
	x := max((width-lipgloss.Width(box))/2, 0)
	y := max((height-lipgloss.Height(box))/2, 0)

	return lipgloss.NewCompositor(
		lipgloss.NewLayer(layout).Z(0),
		lipgloss.NewLayer(box).X(x).Y(y).Z(1),
	).Render()
}

// size clamps a modal's preferred extent to what the terminal can hold, with a
// margin so the layout is still visible around it.
func size(preferWidth, preferHeight, width, height int) (int, int) {
	const margin = 4

	return min(preferWidth, max(width-margin, 1)), min(preferHeight, max(height-margin, 1))
}
