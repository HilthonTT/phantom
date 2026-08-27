package panel

import (
	"strings"

	"github.com/charmbracelet/x/ansi"
)

// Ellipsis is the marker a truncated string ends (or begins) with.
const Ellipsis = "…"

// Width is the rendered width of s, ignoring any ANSI escapes in it.
func Width(s string) int { return ansi.StringWidth(s) }

// Truncate cuts s to at most w columns, ending it with [Ellipsis] if anything
// was dropped. Styling in s is preserved.
func Truncate(s string, w int) string {
	if w <= 0 {
		return ""
	}
	if ansi.StringWidth(s) <= w {
		return s
	}
	return ansi.Truncate(s, w, Ellipsis)
}

// Pad extends s with spaces to exactly w columns, truncating it instead if it
// is already wider.
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

// PadStart is [Pad] with the spaces on the left, for right-aligning a column.
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
