package app

import (
	"strconv"
	"strings"

	tea "charm.land/bubbletea/v2"

	"github.com/HilthonTT/phantom/cli/internal/tui/live"
	"github.com/HilthonTT/phantom/cli/internal/tui/modal"
	"github.com/HilthonTT/phantom/cli/internal/tui/resource"
)

// openActions offers what can be done to the row under the cursor.
func (m Model) openActions() (tea.Model, tea.Cmd) {
	section := m.workspace.Section()
	row, ok := m.workspace.Selected()

	actions := m.live.RowActions(section, row, ok)
	if len(actions) == 0 {
		if live.Served(section) && !m.live.Account.Admin {
			m.notify("Admins only", "Sign in as an admin to act on "+section.String()+".", true)
		}
		return m, nil
	}

	title := section.String()
	if ok && len(row.Cells) > 0 {
		title = row.Cells[0]
	}

	labels := make([]string, len(actions))
	for i, a := range actions {
		labels[i] = a.Label()
	}

	m.actions = actions
	m.menu.Open(title, labels)
	m.modal = modal.Menu

	return m, nil
}

// begin starts an action: asking for its text, then confirming it, then
// running it, skipping whichever steps it does not need.
func (m Model) begin(a live.Action) (tea.Model, tea.Cmd) {
	m = m.closeModal()
	m.acting = a

	if a.NeedsInput() {
		m.modal, m.inputFor = modal.Input, inputAction
		switch a.Kind {
		case live.SetPassword:
			return m, m.input.Open("New password for "+a.Target, "", "the account's devices are signed out", true)
		case live.ReactivateUser:
			return m, m.input.Open("New password for "+a.Target, "", "the account can log in with it", true)
		default:
			return m, m.input.Open("New registration token", "uses days token",
				"all optional: max uses, days valid, the token itself", false)
		}
	}

	return m.confirmAct(a)
}

func (m Model) confirmAct(a live.Action) (tea.Model, tea.Cmd) {
	m.acting = a

	if name := a.TypeToConfirm(); name != "" {
		return m.confirmByName(a, name)
	}

	title, body := a.Confirm()
	if title == "" {
		return m.act(a)
	}

	m.ask(actAction, title, body)

	return m, nil
}

// inputGiven takes the text an action asked for.
func (m Model) inputGiven() (tea.Model, tea.Cmd) {
	a, value := m.acting, strings.TrimSpace(m.input.Value())
	m = m.closeModal()

	switch m.inputFor {
	case inputLogTarget, inputLogSearch:
		return m.logInputGiven(m.inputFor, value), nil
	case inputConfirmName:
		return m.nameGiven(value)
	}

	switch a.Kind {
	case live.SetPassword, live.ReactivateUser:
		if value == "" {
			m.notify("No password given", "The password was left as it was.", true)
			return m, nil
		}
		a.Secret = value

	case live.CreateToken:
		var err error
		if a, err = tokenArgs(a, strings.Fields(value)); err != nil {
			m.notify("Not a token request", err.Error(), true)
			return m, nil
		}
	}

	return m.confirmAct(a)
}

// tokenArgs reads "[uses] [days] [token]" into a token request: the numbers
// first, in that order, and anything else as the token.
func tokenArgs(a live.Action, args []string) (live.Action, error) {
	numbers := 0
	for _, arg := range args {
		n, err := strconv.ParseInt(arg, 10, 64)
		switch {
		case err == nil && n < 0:
			return a, errNegative
		case err == nil && numbers == 0:
			a.Uses = n
			numbers++
		case err == nil && numbers == 1:
			a.Days = n
			numbers++
		case a.Secret == "":
			a.Secret = arg
		default:
			return a, errTokenArgs
		}
	}

	return a, nil
}

// purgeArgs reads a purge's target and its age in days: a room for
// :purgehistory, "remote" or "local" for :cleanmedia.
func purgeArgs(word, target, days string) (live.Action, error) {
	a := live.Action{Kind: live.PurgeHistory, Target: target}
	if word == "cleanmedia" {
		a.Kind = live.PurgeOldMedia
		if target != "remote" && target != "local" {
			return a, errMediaOrigin
		}
	}

	n, err := strconv.ParseInt(days, 10, 64)
	if err != nil || n < 1 {
		return a, errPurgeDays
	}
	a.Days = n

	return a, nil
}

type argError string

func (e argError) Error() string { return string(e) }

const (
	errNegative  argError = "uses and days cannot be negative"
	errTokenArgs argError = "give at most: uses, days, and the token"

	errPurgeDays   argError = "give how many days old, at least one, after what to purge"
	errMediaOrigin argError = "say which media: remote or local"
)

func (m Model) act(a live.Action) (tea.Model, tea.Cmd) {
	return m, live.Act(m.client, a)
}

// acted shows how an action went and fetches what it changed.
func (m Model) acted(msg live.ActedMsg) (tea.Model, tea.Cmd) {
	if msg.Err != nil {
		m.notify(msg.Action.Label()+" failed", msg.Err.Error(), true)
		return m, nil
	}

	m.notify(msg.Action.Label(), msg.Done, false)

	cmds := []tea.Cmd{m.fetchSection(resource.Tasks), m.fetchSection(resource.Audit)}
	for _, s := range msg.Action.Refreshes() {
		cmds = append(cmds, m.fetchSection(s))
	}

	return m, tea.Batch(cmds...)
}

func (m *Model) notify(title, body string, failed bool) {
	m.closeModalInPlace()
	m.notice.Show(title, body, failed)
	m.modal = modal.Notice
}

func (m *Model) closeModalInPlace() {
	*m = m.closeModal()
}

// adminCommand runs one of the prompt's admin commands, with args the words
// after it.
func (m Model) adminCommand(word string, args []string) (tea.Model, tea.Cmd) {
	arg := func(i int) string {
		if i < len(args) {
			return args[i]
		}
		return ""
	}

	var a live.Action
	switch word {
	case "user", "room":
		return m.openRecord(word, arg(0))
	case "inspect":
		if arg(0) == "" {
			m.notify("Missing argument", ":inspect needs a room's ID, name or alias.", true)
			return m, nil
		}
		return m.inspectNamed(arg(0))
	case "passwd":
		a = live.Action{Kind: live.SetPassword, Target: m.userID(arg(0))}
	case "admin":
		a = live.Action{Kind: live.GrantAdmin, Target: m.userID(arg(0))}
	case "unadmin":
		a = live.Action{Kind: live.RevokeAdmin, Target: m.userID(arg(0))}
	case "reactivate":
		a = live.Action{Kind: live.ReactivateUser, Target: m.userID(arg(0))}
	case "deactivate":
		a = live.Action{Kind: live.DeactivateUser, Target: m.userID(arg(0))}
		if arg(1) == "erase" {
			a.Kind = live.EraseUser
		}
	case "token":
		a = live.Action{Kind: live.CreateToken}
		var err error
		if a, err = tokenArgs(a, args); err != nil {
			m.notify("Not a token request", err.Error(), true)
			return m, nil
		}
	case "revoke":
		a = live.Action{Kind: live.RevokeToken, Target: arg(0)}
	case "ban":
		a = live.Action{Kind: live.BanRoom, Target: arg(0)}
	case "unban":
		a = live.Action{Kind: live.UnbanRoom, Target: arg(0)}
	case "shutdown":
		a = live.Action{Kind: live.ShutdownRoom, Target: arg(0)}
	case "purge":
		a = live.Action{Kind: live.DeleteRoom, Target: arg(0)}
	case "rmmedia":
		a = live.Action{Kind: live.DeleteMedia, Target: arg(0)}
	case "purgemedia":
		a = live.Action{Kind: live.PurgeRemoteMedia, Target: arg(0)}
	case "dismiss":
		a = live.Action{Kind: live.DismissReport, Target: arg(0)}
	case "block":
		a = live.Action{Kind: live.BlockServer, Target: arg(0)}
	case "unblock":
		a = live.Action{Kind: live.UnblockServer, Target: arg(0)}
	case "quarantine":
		a = live.Action{Kind: live.QuarantineMedia, Target: arg(0)}
	case "unquarantine":
		a = live.Action{Kind: live.UnquarantineMedia, Target: arg(0)}
	case "purgehistory", "cleanmedia":
		var err error
		if a, err = purgeArgs(word, arg(0), arg(1)); err != nil {
			m.notify("Not a purge", err.Error(), true)
			return m, nil
		}
	case "cancel":
		a = live.Action{Kind: live.CancelTask, Target: arg(0)}
	case "reload":
		a = live.Action{Kind: live.ReloadConfig}
	case "backup":
		a = live.Action{Kind: live.Backup}
	case "":
		return m, nil
	default:
		m.notify("No such command", ":"+word+" is not a command; : lists them.", true)
		return m, nil
	}

	switch {
	case !m.live.Account.Admin:
		m.notify("Admins only", ":"+word+" needs an admin to be signed in.", true)
		return m, nil
	case a.Kind != live.ReloadConfig && a.Kind != live.Backup && a.Kind != live.CreateToken && a.Target == "":
		m.notify("Missing argument", ":"+word+" needs to know what to act on.", true)
		return m, nil

	// The prompt already gave the token's details; the form is for the menu.
	case a.Kind == live.CreateToken:
		return m.confirmAct(a)
	}

	return m.begin(a)
}

// userID turns a bare name into a user ID on the signed-in server.
func (m Model) userID(name string) string {
	if name == "" || strings.Contains(name, ":") {
		return name
	}

	name = strings.TrimPrefix(name, "@")

	server := m.live.Status.ServerName
	if _, domain, ok := strings.Cut(m.live.Account.User, ":"); ok {
		server = domain
	}

	return "@" + name + ":" + server
}

// openRecord opens the users or rooms section on the record named, by ID,
// or for a room by its name or alias too.
func (m Model) openRecord(kind, name string) (tea.Model, tea.Cmd) {
	section := resource.Users
	if kind == "room" {
		section = resource.Rooms
	} else {
		name = m.userID(name)
	}

	m.chatOpen, m.roomOpen, m.focus = false, false, focusWorkspace
	m.workspace.Open(section)

	found := m.workspace.Select(func(r resource.Row) bool {
		if len(r.Ref) > 0 && r.Ref[0] == name {
			return true
		}
		if len(r.Cells) > 0 && r.Cells[0] == name {
			return true
		}
		for _, f := range r.Detail {
			if f.Label == "Alias" && f.Value == name {
				return true
			}
		}
		return false
	})

	if !found && name != "" {
		m.notify("Not found", "No "+kind+" "+name+" is listed.", true)
	}

	return m, m.fetchAdmin(section)
}
