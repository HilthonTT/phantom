package modal

import (
	"strings"

	"charm.land/bubbles/v2/textinput"
	tea "charm.land/bubbletea/v2"

	"github.com/HilthonTT/phantom/cli/internal/tui/panel"
	"github.com/HilthonTT/phantom/cli/internal/tui/theme"
)

// The prompt's preferred extent, borders included.
const (
	promptWidth  = 66
	promptHeight = 8
)

// Command is one entry in the prompt's list of what can be typed.
type Command struct {
	Name  string
	Usage string
}

// Commands is what the prompt offers. Typing one does nothing yet: the prompt
// is here so the shape of the interface is complete, and the handlers are the
// work that follows.
func Commands() []Command {
	return []Command{
		{Name: "room", Usage: "room <alias>          open a room's record"},
		{Name: "user", Usage: "user <id>             open a user's record"},
		{Name: "purge", Usage: "purge <room> <days>   purge history older than"},
		{Name: "block", Usage: "block <server>        stop federating with a server"},
		{Name: "deactivate", Usage: "deactivate <user>     deactivate an account"},
		{Name: "quarantine", Usage: "quarantine <media>    quarantine a media item"},
		{Name: "reload", Usage: "reload                re-read the config file"},
		{Name: "backup", Usage: "backup                start a database backup"},
	}
}

// PromptModel is the command line.
type PromptModel struct {
	theme theme.Theme

	input    textinput.Model
	commands []Command
}

// NewPrompt returns a command prompt.
func NewPrompt(t theme.Theme) PromptModel {
	input := t.Input(" : ", "type a command", t.Palette.Raised)
	input.SetWidth(promptWidth - 8)

	return PromptModel{theme: t, input: input, commands: Commands()}
}

// Focus gives the prompt the keyboard and clears what was last typed.
func (m *PromptModel) Focus() tea.Cmd {
	m.input.SetValue("")

	return m.input.Focus()
}

// Blur takes the keyboard back.
func (m *PromptModel) Blur() { m.input.Blur() }

// Update feeds a message to the input.
func (m *PromptModel) Update(msg tea.Msg) tea.Cmd {
	var cmd tea.Cmd
	m.input, cmd = m.input.Update(msg)

	return cmd
}

// Value is what has been typed.
func (m PromptModel) Value() string { return m.input.Value() }

// matching is the commands whose names start with what has been typed.
func (m PromptModel) matching() []Command {
	word, _, _ := strings.Cut(strings.TrimSpace(m.input.Value()), " ")
	if word == "" {
		return m.commands
	}

	var kept []Command
	for _, c := range m.commands {
		if strings.HasPrefix(c.Name, strings.ToLower(word)) {
			kept = append(kept, c)
		}
	}

	return kept
}

// Render draws the prompt with the commands that still match under it.
func (m PromptModel) Render(width, height int) string {
	w, h := size(promptWidth, promptHeight, width, height)

	p := panel.New(m.theme.ModalConfig(w, h))
	p.SetTitle("Command")

	p.AddLine(m.input.View())
	p.AddDivider()

	matches := m.matching()
	if len(matches) == 0 {
		p.AddLine(m.theme.PromptFailed.Render("   no such command"))
		return p.Render()
	}

	for _, c := range matches {
		if p.Remaining() == 0 {
			break
		}
		p.AddLine(m.theme.ModalHint.Render("  " + panel.Truncate(c.Usage, p.ContentWidth()-2)))
	}

	p.SetInfo("enter runs · esc closes")

	return p.Render()
}
