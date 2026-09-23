package modal

import (
	"charm.land/lipgloss/v2"
)

type Kind int

const (
	None Kind = iota
	Help
	Prompt
	Confirm
)

func Overlay(layout, box string, width, height int) string {
	x := max((width-lipgloss.Width(box))/2, 0)
	y := max((height-lipgloss.Height(box))/2, 0)

	return lipgloss.NewCompositor(
		lipgloss.NewLayer(layout).Z(0),
		lipgloss.NewLayer(box).X(x).Y(y).Z(1),
	).Render()
}

func size(preferWidth, preferHeight, width, height int) (int, int) {
	const margin = 4

	return min(preferWidth, max(width-margin, 1)), min(preferHeight, max(height-margin, 1))
}
