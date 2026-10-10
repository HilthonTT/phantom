package modal

import (
	"strings"

	"charm.land/bubbles/v2/textinput"
	tea "charm.land/bubbletea/v2"

	"github.com/HilthonTT/phantom/cli/internal/tui/panel"
	"github.com/HilthonTT/phantom/cli/internal/tui/theme"
)

const (
	promptWidth  = 66
	promptHeight = 12
)

type Command struct {
	Name  string
	Usage string
}

func Commands() []Command {
	return []Command{
		{Name: "user", Usage: "user <id>                  open a user's record"},
		{Name: "room", Usage: "room <id|#alias>           open a room's record"},
		{Name: "passwd", Usage: "passwd <user>              set a user's password"},
		{Name: "admin", Usage: "admin <user>               make a user an admin"},
		{Name: "unadmin", Usage: "unadmin <user>             revoke a user's admin"},
		{Name: "deactivate", Usage: "deactivate <user> [erase]  deactivate an account"},
		{Name: "token", Usage: "token [uses] [days] [tok]  create a registration token"},
		{Name: "revoke", Usage: "revoke <token>             revoke a registration token"},
		{Name: "ban", Usage: "ban <room>                 stop local users joining"},
		{Name: "unban", Usage: "unban <room>               lift a room's ban"},
		{Name: "shutdown", Usage: "shutdown <room>            evict a room's local members"},
		{Name: "purge", Usage: "purge <room>               delete a room and its data"},
		{Name: "rmmedia", Usage: "rmmedia <mxc://…>          delete a stored file"},
		{Name: "purgemedia", Usage: "purgemedia <server>        delete a server's cached media"},
		{Name: "dismiss", Usage: "dismiss <report id>        close an abuse report"},
		{Name: "reload", Usage: "reload                     re-read the config"},
		{Name: "backup", Usage: "backup                     back the database up"},
		{Name: "join", Usage: "join <#alias|!id>          join a room and open it"},
		{Name: "leave", Usage: "leave                      leave the open room"},
		{Name: "login", Usage: "login                      sign in to the server"},
		{Name: "logout", Usage: "logout                     sign out and forget the session"},
	}
}

type PromptModel struct {
	theme theme.Theme

	input    textinput.Model
	commands []Command
}

func NewPrompt(t theme.Theme) PromptModel {
	input := t.Input(" : ", "type a command", t.Palette.Raised)
	input.SetWidth(promptWidth - 8)

	return PromptModel{theme: t, input: input, commands: Commands()}
}

func (m *PromptModel) Focus() tea.Cmd {
	m.input.SetValue("")

	return m.input.Focus()
}

func (m *PromptModel) Blur() { m.input.Blur() }

func (m *PromptModel) Update(msg tea.Msg) tea.Cmd {
	var cmd tea.Cmd
	m.input, cmd = m.input.Update(msg)

	return cmd
}

func (m PromptModel) Value() string { return m.input.Value() }

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
