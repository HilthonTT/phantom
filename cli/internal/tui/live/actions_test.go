package live

import (
	"testing"

	"github.com/HilthonTT/phantom/cli/internal/tui/resource"
)

func admin(t *testing.T) State {
	t.Helper()

	s := state(t)
	s.Account = Account{User: "@alice:test", Checked: true, Admin: true}

	return s
}

func kinds(actions []Action) []ActionKind {
	out := make([]ActionKind, len(actions))
	for i, a := range actions {
		out[i] = a.Kind
	}

	return out
}

func TestRowActionsFollowTheRecord(t *testing.T) {
	s := admin(t)
	row := func(cells []string, ref ...string) resource.Row { return resource.Row{Cells: cells, Ref: ref} }

	cases := []struct {
		name    string
		section resource.Section
		row     resource.Row
		want    []ActionKind
	}{
		{"an admin user", resource.Users, row([]string{"@a:test", "yes"}, "@a:test", "active"),
			[]ActionKind{SetPassword, RevokeAdmin, DeactivateUser, EraseUser}},
		{"a plain user", resource.Users, row([]string{"@b:test", "no"}, "@b:test", "active"),
			[]ActionKind{SetPassword, GrantAdmin, DeactivateUser, EraseUser}},
		{"a deactivated user", resource.Users, row([]string{"@c:test", "no"}, "@c:test", "deactivated"), nil},
		{"the server user", resource.Users, row([]string{"@phantom:test", "yes"}, "@phantom:test", "server user"), nil},
		{"a device", resource.Devices, row(nil, "@b:test", "DEV"), []ActionKind{SignOutDevice}},
		{"a database token", resource.Tokens, row(nil, "abc", "database"), []ActionKind{CreateToken, RevokeToken}},
		{"the config token", resource.Tokens, row(nil, "***", "config"), []ActionKind{CreateToken}},
		{"a banned room", resource.Rooms, row(nil, "!r:test", "yes"), []ActionKind{UnbanRoom, ShutdownRoom, DeleteRoom}},
		{"a sample row", resource.Rooms, row([]string{"#general"}), nil},
	}

	for _, c := range cases {
		got := kinds(s.RowActions(c.section, c.row, true))
		if len(got) != len(c.want) {
			t.Errorf("%s: actions = %v, want %v", c.name, got, c.want)
			continue
		}
		for i := range c.want {
			if got[i] != c.want[i] {
				t.Errorf("%s: actions = %v, want %v", c.name, got, c.want)
				break
			}
		}
	}
}

func TestOnlyAnAdminGetsActions(t *testing.T) {
	s := state(t)
	row := resource.Row{Cells: []string{"@b:test", "no"}, Ref: []string{"@b:test", "active"}}

	if got := s.RowActions(resource.Users, row, true); got != nil {
		t.Errorf("a non-admin was offered %v", kinds(got))
	}
}

func TestDestructiveActionsAsk(t *testing.T) {
	for _, k := range []ActionKind{DeactivateUser, EraseUser, RevokeAdmin, SignOutDevice, RevokeToken, BanRoom, ShutdownRoom, DeleteRoom} {
		if title, _ := (Action{Kind: k, Target: "x"}).Confirm(); title == "" {
			t.Errorf("%s runs without asking", Action{Kind: k}.Label())
		}
	}
	for _, k := range []ActionKind{CreateToken, UnbanRoom, ReloadConfig, Backup} {
		if title, _ := (Action{Kind: k}).Confirm(); title != "" {
			t.Errorf("%s asks %q", Action{Kind: k}.Label(), title)
		}
	}
}

func TestInsightRowsOfferTheirActions(t *testing.T) {
	s := admin(t)
	cases := map[resource.Section]ActionKind{
		resource.Media:      DeleteMedia,
		resource.Federation: PurgeRemoteMedia,
		resource.Reports:    DismissReport,
	}

	for section, want := range cases {
		got := kinds(s.RowActions(section, resource.Row{Ref: []string{"x"}}, true))
		if len(got) != 1 || got[0] != want {
			t.Errorf("%s: actions = %v, want [%v]", section, got, want)
		}
	}
}
