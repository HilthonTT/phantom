package panel

import (
	"strings"

	"github.com/charmbracelet/x/ansi"
)

const Ellipsis = "…"

func Width(s string) int { return ansi.StringWidth(s) }

func Truncate(s string, w int) string {
	if w <= 0 {
		return ""
	}
	if ansi.StringWidth(s) <= w {
		return s
	}
	return ansi.Truncate(s, w, Ellipsis)
}

func Pad(s string, w int) string {
	width := ansi.StringWidth(s)
	switch {
	case width > w:
		return Truncate(s, w)
	case width == w:
		return s
	default:
		return s + strings.Repeat(" ", w-width)
	}
}

func PadStart(s string, w int) string {
	width := ansi.StringWidth(s)
	switch {
	case width > w:
		return Truncate(s, w)
	case width == w:
		return s
	default:
		return strings.Repeat(" ", w-width) + s
	}
}
