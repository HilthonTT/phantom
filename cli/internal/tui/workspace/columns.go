package workspace

import (
	"strings"

	"github.com/HilthonTT/phantom/cli/internal/tui/panel"
	"github.com/HilthonTT/phantom/cli/internal/tui/resource"
)

const (
	gutter = 2

	marker = 5

	rightMargin = 1

	minFlex = 10
)

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

func flexColumn(cols []resource.Column) int {
	for i, c := range cols {
		if c.Flex {
			return i
		}
	}

	return -1
}

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

func header(cols []resource.Column, w []int) string {
	cells := make([]string, len(cols))
	for i, c := range cols {
		cells[i] = strings.ToUpper(c.Title)
	}

	return row(cells, cols, w)
}

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
