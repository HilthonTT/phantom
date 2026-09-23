package modal

import (
	"strings"

	"charm.land/lipgloss/v2"

	"github.com/HilthonTT/phantom/cli/internal/tui/panel"
	"github.com/HilthonTT/phantom/cli/internal/tui/theme"
)

const (
	confirmWidth  = 58
	confirmHeight = 9
)

type ConfirmModel struct {
	theme theme.Theme

	title  string
	body   string
	accept bool
}

func NewConfirm(t theme.Theme) ConfirmModel {
	return ConfirmModel{theme: t}
}

func (m *ConfirmModel) Ask(title, body string) {
	m.title, m.body, m.accept = title, body, false
}

func (m *ConfirmModel) Toggle() { m.accept = !m.accept }

func (m ConfirmModel) Accepted() bool { return m.accept }

func (m ConfirmModel) Render(width, height int) string {
	w, h := size(confirmWidth, confirmHeight, width, height)

	p := panel.New(m.theme.ModalConfig(w, h))
	p.SetTitle("Confirm")

	p.AddLine("")
	p.AddLine(m.theme.ModalTitle.Render("  " + panel.Truncate(m.title, p.ContentWidth()-2)))
	p.AddLine("")
	p.AddLine(m.theme.ModalHint.Render("  " + panel.Truncate(m.body, p.ContentWidth()-2)))
	p.AddLine("")
	p.AddLine(m.buttons(p.ContentWidth()))

	p.SetInfo("tab switches · enter chooses")

	return p.Render()
}

func (m ConfirmModel) buttons(width int) string {
	confirm, cancel := m.theme.ModalHint, m.theme.ModalHint
	if m.accept {
		confirm = m.theme.ModalConfirm
	} else {
		cancel = m.theme.ModalCancel
	}

	row := lipgloss.JoinHorizontal(lipgloss.Top,
		confirm.Render("  Confirm  "),
		m.theme.Modal.Render("    "),
		cancel.Render("  Cancel  "),
	)

	pad := max((width-lipgloss.Width(row))/2, 0)

	return m.theme.Modal.Render(strings.Repeat(" ", pad)) + row
}
