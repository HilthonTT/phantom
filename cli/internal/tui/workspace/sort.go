package workspace

import (
	"cmp"
	"slices"
	"strconv"
	"strings"

	"github.com/HilthonTT/phantom/cli/internal/tui/resource"
)

// CycleSort steps the open tab's sort through its columns, each ascending
// then descending, and back to the listing's own order after the last.
// The row under the cursor stays under it.
func (m *Model) CycleSort() {
	t := &m.tabs[m.active]
	if len(t.listing.Columns) == 0 {
		return
	}

	selected, had := m.Selected()

	switch {
	case !t.sorted:
		t.sorted, t.sortCol, t.desc = true, 0, false
	case !t.desc:
		t.desc = true
	case t.sortCol+1 < len(t.listing.Columns):
		t.sortCol, t.desc = t.sortCol+1, false
	default:
		t.sorted = false
		fresh := m.newTab(t.Section)
		t.listing.Rows = fresh.listing.Rows
	}

	t.sort()

	if had {
		m.Select(func(r resource.Row) bool { return sameRow(r, selected) })
	}
}

// sort orders the tab's rows by its chosen column, if it has one.
func (t *Tab) sort() {
	if !t.sorted {
		return
	}

	col := t.sortCol
	slices.SortStableFunc(t.listing.Rows, func(a, b resource.Row) int {
		c := compareNatural(sortKey(a, col), sortKey(b, col))
		if t.desc {
			return -c
		}
		return c
	})
}

// sortLabel says how the tab is sorted, for its footer.
func (t Tab) sortLabel() string {
	if !t.sorted || t.sortCol >= len(t.listing.Columns) {
		return t.listing.Sort
	}

	arrow := "ascending"
	if t.desc {
		arrow = "descending"
	}

	return strings.ToLower(t.listing.Columns[t.sortCol].Title) + ", " + arrow
}

func sortKey(r resource.Row, col int) string {
	if key, ok := r.SortKeys[col]; ok {
		return key
	}
	if col < len(r.Cells) {
		return r.Cells[col]
	}

	return ""
}

// compareNatural compares numbers as numbers, ignoring thousands separators,
// and anything else as text without regard to case.
func compareNatural(a, b string) int {
	x, errA := strconv.ParseFloat(strings.ReplaceAll(a, ",", ""), 64)
	y, errB := strconv.ParseFloat(strings.ReplaceAll(b, ",", ""), 64)
	if errA == nil && errB == nil {
		return cmp.Compare(x, y)
	}

	return strings.Compare(strings.ToLower(a), strings.ToLower(b))
}
