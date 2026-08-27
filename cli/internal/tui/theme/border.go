package theme

import "charm.land/lipgloss/v2"

// Border is the rune set every panel is drawn with.
//
// The tees — `MiddleLeft` and `MiddleRight` — are what let a title sit inside
// the top edge (`─┤ Rooms ├─`) and a counter inside the bottom edge, rather
// than costing a line of content each.
func Border() lipgloss.Border {
	return lipgloss.RoundedBorder()
}

// ASCIIBorder is the fallback for terminals without box-drawing characters.
func ASCIIBorder() lipgloss.Border {
	return lipgloss.Border{
		Top: "-", Bottom: "-", Left: "|", Right: "|",
		TopLeft: "+", TopRight: "+", BottomLeft: "+", BottomRight: "+",
		MiddleLeft: "+", MiddleRight: "+",
	}
}
