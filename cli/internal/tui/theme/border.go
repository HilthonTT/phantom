package theme

import "charm.land/lipgloss/v2"

func Border() lipgloss.Border {
	return lipgloss.RoundedBorder()
}

func ASCIIBorder() lipgloss.Border {
	return lipgloss.Border{
		Top: "-", Bottom: "-", Left: "|", Right: "|",
		TopLeft: "+", TopRight: "+", BottomLeft: "+", BottomRight: "+",
		MiddleLeft: "+", MiddleRight: "+",
	}
}
