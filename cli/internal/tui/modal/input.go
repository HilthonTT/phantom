package modal

import (
	"charm.land/bubbles/v2/textinput"
	tea "charm.land/bubbletea/v2"

	"github.com/HilthonTT/phantom/cli/internal/tui/panel"
	"github.com/HilthonTT/phantom/cli/internal/tui/theme"
)

const (
	inputWidth  = 58
	inputHeight = 8
)

// InputModel asks for one line of text, hidden when it is a secret.
type InputModel struct {
	theme theme.Theme

	input textinput.Model

	title string
	hint  string
}

func NewInput(t theme.Theme) InputModel {
	input := t.Input(" › ", "", t.Palette.Raised)
	input.SetWidth(inputWidth - 10)

	return InputModel{theme: t, input: input}
}

// Open asks for a line under title; secret hides what is typed.
func (m *InputModel) Open(title, placeholder, hint string, secret bool) tea.Cmd {
	m.title, m.hint = title, hint
	m.input.SetValue("")
	m.input.Placeholder = placeholder

	m.input.EchoMode = textinput.EchoNormal
	if secret {
		m.input.EchoMode = textinput.EchoPassword
		m.input.EchoCharacter = '•'
	}

	return m.input.Focus()
}

func (m *InputModel) Blur() { m.input.Blur() }

func (m *InputModel) Update(msg tea.Msg) tea.Cmd {
	var cmd tea.Cmd
	m.input, cmd = m.input.Update(msg)

	return cmd
}

func (m InputModel) Value() string { return m.input.Value() }

func (m InputModel) Render(width, height int) string {
	w, h := size(inputWidth, inputHeight, width, height)

	p := panel.New(m.theme.ModalConfig(w, h))
	p.SetTitle("Input")

	p.AddLine("")
	p.AddLine(m.theme.ModalTitle.Render("  " + panel.Truncate(m.title, p.ContentWidth()-2)))
	p.AddLine("")
	p.AddLine(m.input.View())
	p.AddLine("")
	p.AddLine(m.theme.ModalHint.Render("  " + panel.Truncate(m.hint, p.ContentWidth()-2)))

	p.SetInfo("enter accepts · esc cancels")

	return p.Render()
}
