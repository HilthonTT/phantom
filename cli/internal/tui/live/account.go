package live

import (
	"context"
	"errors"

	tea "charm.land/bubbletea/v2"

	"github.com/HilthonTT/phantom/cli/internal/client"
)

// Account is who the console is signed in as.
type Account struct {
	// User is empty when signed out.
	User string

	// Checked is set once the admin API has answered for the session, and
	// Admin is what it said.
	Checked bool
	Admin   bool

	// Err is the admin check failing for another reason than the answer.
	Err error
}

func (a Account) SignedIn() bool { return a.User != "" }

// AuthMsg is a login, or the check of a saved session, finishing.
type AuthMsg struct {
	Session client.Session

	// Resumed is set for the check of a saved session rather than a login.
	Resumed bool

	// Refused is set when the login failed, or the saved session's token is
	// no longer valid; Session is then empty and Err says why.
	Refused bool

	Admin bool
	Err   error
}

// LoggedOutMsg is a logout finishing; Err is the server's refusal, if it
// gave one, though the console is signed out either way.
type LoggedOutMsg struct{ Err error }

// Login signs in, then asks the admin API whether the account is an admin.
func Login(c *client.Client, user, password string) tea.Cmd {
	return func() tea.Msg {
		ctx := context.Background()

		s, err := c.Login(ctx, user, password)
		if err != nil {
			return AuthMsg{Refused: true, Err: err}
		}

		return check(ctx, c, s, false)
	}
}

// Resume checks a saved session, which may have been signed out elsewhere.
func Resume(c *client.Client, s client.Session) tea.Cmd {
	c.Resume(s)

	return func() tea.Msg { return check(context.Background(), c, s, true) }
}

func check(ctx context.Context, c *client.Client, s client.Session, resumed bool) AuthMsg {
	msg := AuthMsg{Session: s, Resumed: resumed}

	_, err := c.WhoAmI(ctx)
	switch {
	case err == nil:
		msg.Admin = true
	case errors.Is(err, client.ErrNotAdmin):
	case client.IsUnknownToken(err):
		c.Drop()
		return AuthMsg{Resumed: resumed, Refused: true, Err: err}
	default:
		msg.Err = err
	}

	return msg
}

func Logout(c *client.Client) tea.Cmd {
	return func() tea.Msg {
		return LoggedOutMsg{Err: c.Logout(context.Background())}
	}
}

// SignIn records a finished login or check.
func (s State) SignIn(msg AuthMsg) State {
	if msg.Refused {
		s.Account = Account{}
		return s
	}

	s.Account = Account{User: msg.Session.UserID, Checked: true, Admin: msg.Admin, Err: msg.Err}

	return s
}

func (s State) SignOut() State {
	s.Account = Account{}
	return s
}

// adminLine is the connection panel's footer: who is signed in.
func (a Account) adminLine() string {
	switch {
	case !a.SignedIn():
		return "not signed in · :login"
	case a.Err != nil:
		return a.User + " · admin check failed"
	case !a.Admin:
		return a.User + " · not an admin"
	default:
		return a.User + " · admin"
	}
}
