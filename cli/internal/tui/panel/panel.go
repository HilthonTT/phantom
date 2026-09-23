package panel

import (
	"image/color"
	"strings"

	"charm.land/lipgloss/v2"
)

const (
	borderThickness = 2

	titleAffix = 2
	infoAffix  = 2

	titlePadding = 2

	minTitleWidth = titleAffix + titlePadding + 1

	minInfoWidth = infoAffix + titlePadding + 1

	titleIndent = 1
)

type Config struct {
	Width  int
	Height int

	Border   lipgloss.Border
	BorderFG color.Color
	BorderBG color.Color

	ContentFG color.Color
	ContentBG color.Color
}

type Panel struct {
	cfg Config

	title string
	info  []string

	lines    []string
	dividers []int
}

func New(cfg Config) *Panel {
	cfg.Width = max(cfg.Width, 0)
	cfg.Height = max(cfg.Height, 0)

	return &Panel{cfg: cfg}
}

func (p *Panel) ContentWidth() int { return max(p.cfg.Width-borderThickness, 0) }

func (p *Panel) ContentHeight() int { return max(p.cfg.Height-borderThickness, 0) }

func (p *Panel) SetTitle(s string) { p.title = s }

func (p *Panel) SetInfo(items ...string) { p.info = items }

func (p *Panel) AddLine(s string) {
	p.lines = append(p.lines, Truncate(s, p.ContentWidth()))
}

func (p *Panel) AddLines(ss ...string) {
	for _, s := range ss {
		p.AddLine(s)
	}
}

func (p *Panel) AddDivider() {
	rule := lipgloss.NewStyle().
		Foreground(p.cfg.BorderFG).
		Background(p.cfg.ContentBG).
		Render(strings.Repeat(p.cfg.Border.Top, p.ContentWidth()))

	p.dividers = append(p.dividers, len(p.lines))
	p.lines = append(p.lines, rule)
}

func (p *Panel) Remaining() int { return max(p.ContentHeight()-len(p.lines), 0) }

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

const ansiEscape = '\x1b'

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
