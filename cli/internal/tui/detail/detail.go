package detail

import (
	"github.com/HilthonTT/phantom/cli/internal/tui/panel"
	"github.com/HilthonTT/phantom/cli/internal/tui/resource"
	"github.com/HilthonTT/phantom/cli/internal/tui/theme"
)

const Indent = "  "

func Line(t theme.Theme, f resource.Field, labelWidth, width int) string {
	label := panel.Pad(panel.Truncate(f.Label, labelWidth), labelWidth)
	room := max(width-labelWidth-panel.Width(Indent)-1, 1)

	return t.Muted.Render(Indent+label) +
		t.ForState(f.Emphasis).Render(panel.Truncate(f.Value, room))
}

func Fill(p *panel.Panel, t theme.Theme, fields []resource.Field, labelWidth int) int {
	for i, f := range fields {
		if p.Remaining() == 0 {
			return len(fields) - i
		}
		p.AddLine(Line(t, f, labelWidth, p.ContentWidth()))
	}

	return 0
}
