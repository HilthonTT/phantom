package workspace

import (
	"strings"

	"github.com/HilthonTT/phantom/cli/internal/tui/panel"
	"github.com/HilthonTT/phantom/cli/internal/tui/resource"
)

// Column geometry, in terminal columns.
const (
	// gutter is the gap between two columns of a listing.
	gutter = 2

	// marker is the width of the cursor-and-mark column down the left of every
	// row: a space, the cursor glyph, a space, the mark glyph, and a space.
	marker = 5

	// rightMargin keeps the last column off the panel's border.
	rightMargin = 1

	// minFlex is the narrowest the flexible column may become before columns
	// start being dropped to make room for it.
	minFlex = 10
)

// widths distributes the available width over the listing's columns.
//
// Fixed columns get what they asked for and the flexible one gets the rest,
// which is normally the name or the message — the column a reader scans, and
// the one worth the space.
//
// Where they do not all fit, columns are dropped from the right rather than
// squeezed: half a column of digits says less than no column at all, and a
// listing whose rows are cut off mid-cell is unreadable in a way a shorter
// listing is not. A dropped column comes back as a width of zero, and is
// skipped by [header] and [row].
func widths(cols []resource.Column, available int) []int {
	out := make([]int, len(cols))
	flex := flexColumn(cols)

	visible := make([]int, 0, len(cols))
	for i := range cols {
		visible = append(visible, i)
	}

	for len(visible) > 1 && required(cols, visible, flex) > available {
		drop := len(visible) - 1
		if visible[drop] == flex {
			drop--
		}
		visible = append(visible[:drop], visible[drop+1:]...)
	}

	spare := available - gutter*max(len(visible)-1, 0)
	for _, i := range visible {
		if i == flex {
			continue
		}
		out[i] = cols[i].Width
		spare -= cols[i].Width
	}

	if flex >= 0 {
		out[flex] = max(spare, minFlex)
	}

	return out
}

// flexColumn is the index of the one column marked flexible, or -1 where a
// listing has none and is simply left-packed.
func flexColumn(cols []resource.Column) int {
	for i, c := range cols {
		if c.Flex {
			return i
		}
	}

	return -1
}

// required is the width the given columns need between them.
func required(cols []resource.Column, visible []int, flex int) int {
	need := gutter * max(len(visible)-1, 0)

	for _, i := range visible {
		if i == flex {
			need += minFlex
			continue
		}
		need += cols[i].Width
	}

	return need
}

// header is the column titles, in the same geometry as the rows under them.
func header(cols []resource.Column, w []int) string {
	cells := make([]string, len(cols))
	for i, c := range cols {
		cells[i] = strings.ToUpper(c.Title)
	}

	return row(cells, cols, w)
}

// row lays out one row's cells, each padded or truncated to its column and
// aligned as that column asks. Columns [widths] dropped are skipped, along
// with the gutter that would have preceded them.
func row(cells []string, cols []resource.Column, w []int) string {
	var out strings.Builder
	written := 0

	for i, c := range cols {
		if i >= len(w) || w[i] == 0 {
			continue
		}
		if written > 0 {
			out.WriteString(strings.Repeat(" ", gutter))
		}
		written++

		cell := ""
		if i < len(cells) {
			cell = cells[i]
		}

		if c.Right {
			out.WriteString(panel.PadStart(cell, w[i]))
			continue
		}
		out.WriteString(panel.Pad(cell, w[i]))
	}

	return out.String()
}
