package panel

import (
	"strings"
	"testing"

	"charm.land/lipgloss/v2"
)

func testPanel(width, height int) *Panel {
	return New(Config{
		Width:  width,
		Height: height,
		Border: lipgloss.RoundedBorder(),
	})
}

// A panel is the unit the layout is joined from, so it has to be exactly the
// size it was asked for whatever it was given to hold.
func TestPanelIsExactlyItsConfiguredSize(t *testing.T) {
	cases := []struct {
		name          string
		width, height int
		lines         []string
	}{
		{name: "empty", width: 20, height: 6},
		{name: "underfull", width: 20, height: 6, lines: []string{"one", "two"}},
		{name: "overfull", width: 20, height: 4, lines: []string{"a", "b", "c", "d", "e", "f"}},
		{name: "over-wide line", width: 12, height: 3, lines: []string{strings.Repeat("x", 40)}},
		{name: "single column", width: 1, height: 3, lines: []string{"x"}},
	}

	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			p := testPanel(c.width, c.height)
			p.SetTitle("Title")
			p.SetInfo("1/2")
			p.AddLines(c.lines...)

			out := p.Render()

			if got := lipgloss.Height(out); got != c.height {
				t.Errorf("height = %d, want %d\n%s", got, c.height, out)
			}
			if got := lipgloss.Width(out); got != c.width {
				t.Errorf("width = %d, want %d\n%s", got, c.width, out)
			}
		})
	}
}

// The title and the info items live in the border rather than costing a line
// of content, which is the whole reason for the custom border assembly.
func TestBorderCarriesTitleAndInfo(t *testing.T) {
	p := testPanel(30, 5)
	p.SetTitle("Rooms")
	p.SetInfo("3/12")
	p.AddLine("content")

	lines := strings.Split(p.Render(), "\n")

	if !strings.Contains(lines[0], "Rooms") {
		t.Errorf("top border does not carry the title: %q", lines[0])
	}
	if !strings.Contains(lines[len(lines)-1], "3/12") {
		t.Errorf("bottom border does not carry the info: %q", lines[len(lines)-1])
	}
	if !strings.Contains(lines[1], "content") {
		t.Errorf("first content line should be the content, got %q", lines[1])
	}
}

// A title too long for the edge is truncated rather than pushing the corner
// out and making the panel wider than it was configured to be.
func TestOverlongTitleDoesNotWidenThePanel(t *testing.T) {
	p := testPanel(20, 4)
	p.SetTitle(strings.Repeat("long ", 20))

	if got := lipgloss.Width(p.Render()); got != 20 {
		t.Errorf("width = %d, want 20", got)
	}
}

// A divider puts a tee in each side border on its own row, so the rule meets
// the frame instead of stopping short of it.
func TestDividerMeetsTheSideBorders(t *testing.T) {
	p := testPanel(14, 6)
	p.AddLine("above")
	p.AddDivider()
	p.AddLine("below")

	lines := strings.Split(p.Render(), "\n")
	divider := lines[2]

	if !strings.HasPrefix(divider, "├") || !strings.HasSuffix(divider, "┤") {
		t.Errorf("divider row is not teed into the borders: %q", divider)
	}
}

func TestRemainingCountsTheLinesLeft(t *testing.T) {
	p := testPanel(10, 6)
	if got := p.Remaining(); got != 4 {
		t.Fatalf("Remaining on an empty panel = %d, want 4", got)
	}

	p.AddLines("a", "b")
	if got := p.Remaining(); got != 2 {
		t.Errorf("Remaining after two lines = %d, want 2", got)
	}

	p.AddLines("c", "d", "e", "f")
	if got := p.Remaining(); got != 0 {
		t.Errorf("Remaining on a full panel = %d, want 0", got)
	}
}
