package modal

import (
	"strings"

	"github.com/charmbracelet/x/ansi"

	"github.com/HilthonTT/phantom/cli/internal/tui/panel"
	"github.com/HilthonTT/phantom/cli/internal/tui/theme"
)

const noticeWidth = 62

// NoticeModel tells the outcome of something, done or failed.
type NoticeModel struct {
	theme theme.Theme

	title  string
	body   string
	failed bool
}

func NewNotice(t theme.Theme) NoticeModel { return NoticeModel{theme: t} }

func (m *NoticeModel) Show(title, body string, failed bool) {
	m.title, m.body, m.failed = title, body, failed
}

func (m NoticeModel) Render(width, height int) string {
	lines := strings.Split(ansi.Wrap(m.body, noticeWidth-8, ""), "\n")
	w, h := size(noticeWidth, len(lines)+6, width, height)

	p := panel.New(m.theme.ModalConfig(w, h))
	p.SetTitle("Done")

	title := m.theme.ModalTitle
	if m.failed {
		p.SetTitle("Failed")
		title = m.theme.PromptFailed
	}

	p.AddLine("")
	p.AddLine(title.Render("  " + panel.Truncate(m.title, p.ContentWidth()-2)))
	p.AddLine("")
	for _, line := range lines {
		if p.Remaining() < 1 {
			break
		}
		p.AddLine(m.theme.ModalHint.Render("  " + panel.Truncate(line, p.ContentWidth()-2)))
	}

	p.SetInfo("enter or esc closes")

	return p.Render()
}
