package taskbar

import (
	"image/color"

	"charm.land/bubbles/v2/progress"

	"github.com/HilthonTT/phantom/cli/internal/tui/resource"
	"github.com/HilthonTT/phantom/cli/internal/tui/sample"
	"github.com/HilthonTT/phantom/cli/internal/tui/theme"
)

const rowsPerTask = 3

type Model struct {
	theme  theme.Theme
	glyphs theme.Glyphs

	tasks  []resource.Task
	cursor int
	top    int

	width  int
	height int

	bars map[resource.State]progress.Model
}

func New(t theme.Theme, g theme.Glyphs) Model {
	m := Model{
		theme:  t,
		glyphs: g,
		tasks:  sample.Tasks(),
		bars:   make(map[resource.State]progress.Model, 4),
	}

	for state, colour := range map[resource.State]color.Color{
		resource.Running: t.Palette.Info,
		resource.Done:    t.Palette.Success,
		resource.Failed:  t.Palette.Danger,
		resource.Held:    t.Palette.Warning,
	} {
		m.bars[state] = progress.New(
			progress.WithColors(colour),
			progress.WithFillCharacters(g.ProgressFull, g.ProgressEmpty),
			progress.WithoutPercentage(),
		)
	}

	return m
}

func (m *Model) SetSize(width, height int) {
	m.width, m.height = width, height
	m.clampScroll()
}

func (m Model) Selected() (resource.Task, bool) {
	if len(m.tasks) == 0 {
		return resource.Task{}, false
	}

	return m.tasks[m.cursor], true
}

func (m *Model) MoveUp()   { m.moveTo(m.cursor - 1) }
func (m *Model) MoveDown() { m.moveTo(m.cursor + 1) }

func (m *Model) moveTo(i int) {
	m.cursor = min(max(i, 0), max(len(m.tasks)-1, 0))
	m.clampScroll()
}

func (m *Model) clampScroll() {
	visible := m.visibleTasks()

	if m.cursor < m.top {
		m.top = m.cursor
	}
	if m.cursor >= m.top+visible {
		m.top = m.cursor - visible + 1
	}

	m.top = min(max(m.top, 0), max(len(m.tasks)-visible, 0))
}

func (m Model) visibleTasks() int {
	return max((m.height-2)/rowsPerTask, 1)
}
