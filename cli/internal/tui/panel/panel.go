// Package panel draws the bordered box every part of the interface lives in.
//
// A panel is a fixed rectangle: it always renders exactly [Config.Height]
// lines of exactly [Config.Width] columns, whatever it was given to hold, so
// the panels of a layout can be joined without any of them pushing the others
// around.
//
// What makes the box more than a rectangle is that its border carries content.
// A title sits in the top edge and a counter or two in the bottom edge —
// `─┤ Rooms ├─` and `─┤ 3/12 ├─` — which costs no content line, and dividers
// sit in the side edges so a panel can be sectioned without a rule that runs
// into the border. This is the same trick superfile uses, and it is why its
// panels read as dense without looking cramped.
package panel

import (
	"image/color"
	"strings"

	"charm.land/lipgloss/v2"
)

// Sizes of the border furniture, in columns.
const (
	// borderThickness is the two edge columns, or rows, a border occupies.
	borderThickness = 2

	// titleAffix is the two tees around a border title, and infoAffix the same
	// around one bottom-edge info item.
	titleAffix = 2
	infoAffix  = 2

	// titlePadding is the space either side of a border title's or an info
	// item's text.
	titlePadding = 2

	// minTitleWidth is the narrowest top edge a title is drawn in at all;
	// below it the tees and padding would leave no room for a character.
	minTitleWidth = titleAffix + titlePadding + 1

	// minInfoWidth is the same for one bottom-edge info item.
	minInfoWidth = infoAffix + titlePadding + 1

	// titleIndent is how far along the top edge the title starts.
	titleIndent = 1
)

// Config is the fixed shape and colouring of a panel.
type Config struct {
	// Width and Height include the border.
	Width  int
	Height int

	Border   lipgloss.Border
	BorderFG color.Color
	BorderBG color.Color

	ContentFG color.Color
	ContentBG color.Color
}

// Panel accumulates lines and renders them inside a border. Build one with
// [New], fill it with [Panel.AddLine], and finish with [Panel.Render].
type Panel struct {
	cfg Config

	title string
	info  []string

	lines    []string
	dividers []int
}

// New returns an empty panel of the configured size. A width or height too
// small for a border is clamped rather than rejected: a panel is drawn from a
// terminal size nobody promised was reasonable, and a missing box is a better
// failure than a panic.
func New(cfg Config) *Panel {
	cfg.Width = max(cfg.Width, 0)
	cfg.Height = max(cfg.Height, 0)

	return &Panel{cfg: cfg}
}

// ContentWidth is how many columns a caller may fill per line.
func (p *Panel) ContentWidth() int { return max(p.cfg.Width-borderThickness, 0) }

// ContentHeight is how many lines a caller may add before they are dropped.
func (p *Panel) ContentHeight() int { return max(p.cfg.Height-borderThickness, 0) }

// SetTitle puts s in the top edge of the border. Styling is not carried into
// the border, so pass plain text.
func (p *Panel) SetTitle(s string) { p.title = s }

// SetInfo puts each item in the bottom edge of the border, in order, ending at
// the bottom-right corner. Two or three short items is what fits; more are
// each given a proportionally narrower slot.
func (p *Panel) SetInfo(items ...string) { p.info = items }

// AddLine appends one line of content, truncated to the panel's width.
func (p *Panel) AddLine(s string) {
	p.lines = append(p.lines, Truncate(s, p.ContentWidth()))
}

// AddLines appends each of ss with [Panel.AddLine].
func (p *Panel) AddLines(ss ...string) {
	for _, s := range ss {
		p.AddLine(s)
	}
}

// AddDivider appends a horizontal rule, and marks the side borders so the rule
// meets them as a tee rather than stopping short of them.
//
// The rule is drawn in the border's colour rather than the content's: it is
// part of the frame, and reads as one continuous line with the tees it runs
// into.
func (p *Panel) AddDivider() {
	rule := lipgloss.NewStyle().
		Foreground(p.cfg.BorderFG).
		Background(p.cfg.ContentBG).
		Render(strings.Repeat(p.cfg.Border.Top, p.ContentWidth()))

	p.dividers = append(p.dividers, len(p.lines))
	p.lines = append(p.lines, rule)
}

// Remaining is the number of lines that still fit.
func (p *Panel) Remaining() int { return max(p.ContentHeight()-len(p.lines), 0) }

// Render draws the panel. The result is exactly Height lines of exactly Width
// columns.
func (p *Panel) Render() string {
	if p.cfg.Width <= 0 || p.cfg.Height <= 0 {
		return ""
	}

	if p.cfg.Width < borderThickness || p.cfg.Height < borderThickness {
		return strings.Join(p.body(p.cfg.Width, p.cfg.Height), "\n")
	}

	body := p.body(p.ContentWidth(), p.ContentHeight())

	return lipgloss.NewStyle().
		Border(p.border()).
		BorderForeground(p.cfg.BorderFG).
		BorderBackground(p.cfg.BorderBG).
		Render(strings.Join(body, "\n"))
}

// body is the content, padded to exactly h lines of exactly w columns.
//
// Each line is padded with a separately styled run of spaces rather than by
// styling the finished line: a line that already carries styling ends with an
// SGR reset, and anything appended after that reset would fall back to the
// terminal's own colours and leave a notch in the panel's background.
func (p *Panel) body(w, h int) []string {
	content := lipgloss.NewStyle().
		Foreground(p.cfg.ContentFG).
		Background(p.cfg.ContentBG)

	fill := func(n int) string {
		if n <= 0 {
			return ""
		}
		return content.Render(strings.Repeat(" ", n))
	}

	body := make([]string, 0, h)

	for _, line := range p.lines {
		if len(body) == h {
			break
		}

		line = Truncate(line, w)

		if !strings.ContainsRune(line, ansiEscape) {
			line = content.Render(line)
		}

		body = append(body, line+fill(w-Width(line)))
	}

	for len(body) < h {
		body = append(body, fill(w))
	}

	return body
}

// ansiEscape starts every SGR sequence, and so marks a line that carries its
// own styling.
const ansiEscape = '\x1b'

// border returns the panel's border with the title, the info items and the
// dividers written into its edges.
//
// lipgloss fills each edge by cycling through the runes of the corresponding
// `Border` field, so an edge string of exactly the right length is laid down
// once, verbatim. That is what lets a whole title live in `Top`.
func (p *Panel) border() lipgloss.Border {
	b := p.cfg.Border

	if top, ok := p.topEdge(b); ok {
		b.Top = top
	}
	if bottom, ok := p.bottomEdge(b); ok {
		b.Bottom = bottom
	}
	if len(p.dividers) > 0 {
		b.Left, b.Right = p.sideEdges(b)
	}

	return b
}

// topEdge is the top border with the title set into it, or ok=false where
// there is no title or no room for one.
func (p *Panel) topEdge(b lipgloss.Border) (string, bool) {
	width := p.ContentWidth()
	if p.title == "" || width < minTitleWidth {
		return "", false
	}

	title := Truncate(p.title, width-titleAffix-titlePadding)
	fill := width - titleAffix - titlePadding - Width(title)

	indent := ""
	if fill > titleIndent {
		indent = strings.Repeat(b.Top, titleIndent)
		fill -= titleIndent
	}

	return indent + b.MiddleRight + " " + title + " " + b.MiddleLeft +
		strings.Repeat(b.Top, fill), true
}

// bottomEdge is the bottom border with the info items set into it, right
// aligned, or ok=false where there are none or no room for them.
func (p *Panel) bottomEdge(b lipgloss.Border) (string, bool) {
	width := p.ContentWidth()
	if len(p.info) == 0 || width < len(p.info)*minInfoWidth {
		return "", false
	}

	share := width/len(p.info) - infoAffix - titlePadding

	var items strings.Builder
	for _, item := range p.info {
		items.WriteString(b.MiddleRight + " " + Truncate(item, share) + " " + b.MiddleLeft)
	}

	fill := width - Width(items.String())

	return strings.Repeat(b.Bottom, fill) + items.String(), true
}

// sideEdges are the left and right borders with a tee at each divider row.
func (p *Panel) sideEdges(b lipgloss.Border) (string, string) {
	var left, right strings.Builder

	next := 0
	for row := range p.ContentHeight() {
		if next < len(p.dividers) && p.dividers[next] == row {
			next++
			left.WriteString(b.MiddleLeft)
			right.WriteString(b.MiddleRight)
			continue
		}
		left.WriteString(b.Left)
		right.WriteString(b.Right)
	}

	return left.String(), right.String()
}
