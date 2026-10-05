package modal

import (
	"charm.land/bubbles/v2/textinput"
	tea "charm.land/bubbletea/v2"

	"github.com/HilthonTT/phantom/cli/internal/tui/panel"
	"github.com/HilthonTT/phantom/cli/internal/tui/theme"
)

const (
	loginWidth  = 58
	loginHeight = 10
)

// LoginModel asks for the account to sign in with: a user and a password,
// tab moving between them.
type LoginModel struct {
	theme theme.Theme

	user     textinput.Model
	password textinput.Model
	onUser   bool

	server string
	err    string
	busy   bool
}

func NewLogin(t theme.Theme) LoginModel {
	user := t.Input(" user     ", "@admin:server or admin", t.Palette.Raised)
	password := t.Input(" password ", "", t.Palette.Raised)
	password.EchoMode = textinput.EchoPassword
	password.EchoCharacter = '•'

	for _, in := range []*textinput.Model{&user, &password} {
		in.SetWidth(loginWidth - 18)
	}

	return LoginModel{theme: t, user: user, password: password, onUser: true}
}

// Open shows the form for server, keeping a user typed before and clearing
// the password, with err explaining why it opened, if anything went wrong.
func (m *LoginModel) Open(server, err string) tea.Cmd {
	m.server, m.err, m.busy = server, err, false
	m.password.SetValue("")

	if m.user.Value() == "" {
		return m.focus(true)
	}

	return m.focus(false)
}

func (m *LoginModel) Blur() {
	m.user.Blur()
	m.password.Blur()
}

func (m *LoginModel) Toggle() tea.Cmd { return m.focus(!m.onUser) }

func (m *LoginModel) focus(user bool) tea.Cmd {
	m.onUser = user
	m.Blur()

	if user {
		return m.user.Focus()
	}

	return m.password.Focus()
}

func (m *LoginModel) Update(msg tea.Msg) tea.Cmd {
	if m.busy {
		return nil
	}

	var cmd tea.Cmd
	if m.onUser {
		m.user, cmd = m.user.Update(msg)
	} else {
		m.password, cmd = m.password.Update(msg)
	}

	return cmd
}

// Submit reports the user and password when both are filled in, marking the
// form busy until Fail or a close; with one empty it moves there instead.
func (m *LoginModel) Submit() (user, password string, ok bool, cmd tea.Cmd) {
	if m.busy {
		return "", "", false, nil
	}

	switch {
	case m.user.Value() == "":
		return "", "", false, m.focus(true)
	case m.password.Value() == "":
		return "", "", false, m.focus(false)
	}

	m.busy, m.err = true, ""

	return m.user.Value(), m.password.Value(), true, nil
}

// Fail shows why the login did not go through and lets the user retry.
func (m *LoginModel) Fail(err string) tea.Cmd {
	m.busy, m.err = false, err
	m.password.SetValue("")

	return m.focus(false)
}

func (m LoginModel) Busy() bool { return m.busy }

func (m LoginModel) Render(width, height int) string {
	w, h := size(loginWidth, loginHeight, width, height)

	p := panel.New(m.theme.ModalConfig(w, h))
	p.SetTitle("Sign in")

	p.AddLine("")
	p.AddLine(m.theme.ModalTitle.Render("  " + panel.Truncate("Sign in to "+m.server, p.ContentWidth()-2)))
	p.AddLine("")
	p.AddLine(m.user.View())
	p.AddLine(m.password.View())
	p.AddLine("")

	switch {
	case m.busy:
		p.AddLine(m.theme.ModalHint.Render("  signing in…"))
	case m.err != "":
		p.AddLine(m.theme.PromptFailed.Render("  " + panel.Truncate(m.err, p.ContentWidth()-2)))
	default:
		p.AddLine(m.theme.ModalHint.Render("  an admin account manages the server"))
	}

	p.SetInfo("tab switches · enter signs in · esc closes")

	return p.Render()
}
