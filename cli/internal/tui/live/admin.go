package live

import (
	"context"
	"time"

	tea "charm.land/bubbletea/v2"

	"github.com/HilthonTT/phantom/cli/internal/client"
	"github.com/HilthonTT/phantom/cli/internal/tui/listings"
	"github.com/HilthonTT/phantom/cli/internal/tui/resource"
)

// AdminSections are the sections the admin API can fill; the rest stay sample
// data.
var AdminSections = []resource.Section{
	resource.Overview,
	resource.Users,
	resource.Devices,
	resource.Tokens,
	resource.Rooms,
	resource.Appservices,
	resource.Settings,
	resource.Tasks,
	resource.Services,
	resource.Federation,
	resource.Media,
	resource.Logs,
	resource.Reports,
}

func Served(s resource.Section) bool {
	for _, served := range AdminSections {
		if served == s {
			return true
		}
	}

	return false
}

// Admin is what the admin API has answered so far for the signed-in admin.
type Admin struct {
	// Gen numbers sign-ins, so an answer for an earlier account is dropped.
	Gen int

	Stats *client.Stats

	// Devices is the device count, once the devices are known, for the
	// overview.
	Devices *int

	// Tasks are the server's long operations, for the taskbar; nil until
	// they are known.
	Tasks []resource.Task

	// Services tallies the server's services, for the overview, once known.
	Services *ServiceCounts

	// Peers tallies the servers this one shares rooms with, once known.
	Peers *PeerCounts

	listings map[resource.Section]resource.Listing
	errs     map[resource.Section]error
}

// PeerCounts tallies the servers this one shares rooms with.
type PeerCounts struct {
	Known, Reachable, Failing int
}

// ServiceCounts tallies the server's services.
type ServiceCounts struct {
	Total, Running, Failed int
}

// LogLines is how many of the server's recent log lines the Logs section
// asks for.
const LogLines = 500

// AdminMsg is one section's answer arriving.
type AdminMsg struct {
	Gen     int
	Section resource.Section

	Listing  resource.Listing
	Stats    *client.Stats
	Devices  *int
	Tasks    []resource.Task
	Services *ServiceCounts
	Peers    *PeerCounts

	Err error
}

// FetchAdmin asks the admin API for a section.
func FetchAdmin(c *client.Client, gen int, section resource.Section) tea.Cmd {
	return func() tea.Msg {
		ctx := context.Background()
		now := time.Now()
		msg := AdminMsg{Gen: gen, Section: section}

		switch section {
		case resource.Overview:
			stats, err := c.Stats(ctx)
			msg.Stats, msg.Err = &stats, err

		case resource.Users:
			users, err := c.Users(ctx)
			msg.Listing, msg.Err = listings.Users(users, now), err

		case resource.Devices:
			devices, err := c.Devices(ctx)
			n := len(devices)
			msg.Listing, msg.Devices, msg.Err = listings.Devices(devices, now), &n, err

		case resource.Tokens:
			tokens, err := c.RegistrationTokens(ctx)
			msg.Listing, msg.Err = listings.Tokens(tokens, now), err

		case resource.Rooms:
			rooms, err := c.Rooms(ctx)
			msg.Listing, msg.Err = listings.Rooms(rooms), err

		case resource.Appservices:
			appservices, err := c.Appservices(ctx)
			msg.Listing, msg.Err = listings.Appservices(appservices), err

		case resource.Settings:
			settings, err := c.Settings(ctx)
			msg.Listing, msg.Err = listings.Settings(settings), err

		case resource.Tasks:
			tasks, err := c.Tasks(ctx)
			msg.Listing, msg.Tasks, msg.Err = listings.Tasks(tasks, now), listings.Taskbar(tasks), err

		case resource.Services:
			services, err := c.Services(ctx)
			var counts ServiceCounts
			counts.Total, counts.Running, counts.Failed = listings.ServiceCounts(services)
			msg.Listing, msg.Services, msg.Err = listings.Services(services, now), &counts, err

		case resource.Federation:
			peers, err := c.Federation(ctx)
			var counts PeerCounts
			counts.Known, counts.Reachable, counts.Failing = listings.PeerCounts(peers)
			msg.Listing, msg.Peers, msg.Err = listings.Federation(peers, now), &counts, err

		case resource.Media:
			media, err := c.Media(ctx)
			msg.Listing, msg.Err = listings.Media(media, now), err

		case resource.Logs:
			lines, err := c.Logs(ctx, LogLines)
			msg.Listing, msg.Err = listings.Logs(lines, now), err

		case resource.Reports:
			reports, err := c.Reports(ctx)
			msg.Listing, msg.Err = listings.Reports(reports, now), err

		default:
			return nil
		}

		return msg
	}
}

// FetchAllAdmin asks for every section the admin API serves.
func FetchAllAdmin(c *client.Client, gen int) tea.Cmd {
	cmds := make([]tea.Cmd, 0, len(AdminSections))
	for _, s := range AdminSections {
		cmds = append(cmds, FetchAdmin(c, gen, s))
	}

	return tea.Batch(cmds...)
}

// StartAdmin forgets the last admin's answers, for a new sign-in or a
// sign-out, returning the generation answers must now carry.
func (s State) StartAdmin() (State, int) {
	s.Admin = Admin{Gen: s.Admin.Gen + 1}
	return s, s.Admin.Gen
}

// TakeAdmin records an answer, reporting whether it belonged to this sign-in.
func (s State) TakeAdmin(msg AdminMsg) (State, bool) {
	if msg.Gen != s.Admin.Gen {
		return s, false
	}

	a := s.Admin
	a.listings = clone(a.listings)
	a.errs = clone(a.errs)

	if msg.Err != nil {
		a.errs[msg.Section] = msg.Err
		s.Admin = a
		return s, true
	}

	delete(a.errs, msg.Section)
	switch msg.Section {
	case resource.Overview:
		a.Stats = msg.Stats
	default:
		a.listings[msg.Section] = msg.Listing
		if msg.Devices != nil {
			a.Devices = msg.Devices
		}
		if msg.Section == resource.Tasks {
			a.Tasks = msg.Tasks
		}
		if msg.Services != nil {
			a.Services = msg.Services
		}
		if msg.Peers != nil {
			a.Peers = msg.Peers
		}
	}

	s.Admin = a
	return s, true
}

// AdminErr is why a section's last fetch failed, if it did.
func (s State) AdminErr(section resource.Section) error { return s.Admin.errs[section] }

func clone[K comparable, V any](m map[K]V) map[K]V {
	out := make(map[K]V, len(m)+1)
	for k, v := range m {
		out[k] = v
	}

	return out
}

// RowActions are what an admin can do to the record a row shows, and the
// section-wide actions; none for a sample row.
func (s State) RowActions(section resource.Section, row resource.Row, ok bool) []Action {
	if !s.Account.Admin {
		return nil
	}

	ref := func(i int) string {
		if !ok || len(row.Ref) <= i {
			return ""
		}
		return row.Ref[i]
	}

	switch section {
	case resource.Users:
		user, state := ref(0), ref(1)
		if user == "" || state != "active" {
			return nil
		}
		admin := Action{Kind: GrantAdmin, Target: user}
		if len(row.Cells) > 1 && row.Cells[1] == "yes" {
			admin.Kind = RevokeAdmin
		}
		return []Action{
			{Kind: SetPassword, Target: user},
			admin,
			{Kind: DeactivateUser, Target: user},
			{Kind: EraseUser, Target: user},
		}

	case resource.Devices:
		if ref(1) == "" {
			return nil
		}
		return []Action{{Kind: SignOutDevice, Target: ref(0), Device: ref(1)}}

	case resource.Tokens:
		actions := []Action{{Kind: CreateToken}}
		if ref(1) == "database" {
			actions = append(actions, Action{Kind: RevokeToken, Target: ref(0)})
		}
		return actions

	case resource.Rooms:
		room := ref(0)
		if room == "" {
			return nil
		}
		ban := Action{Kind: BanRoom, Target: room}
		if ref(1) == "yes" {
			ban.Kind = UnbanRoom
		}
		return []Action{ban, {Kind: ShutdownRoom, Target: room}, {Kind: DeleteRoom, Target: room}}

	case resource.Overview, resource.Settings:
		return []Action{{Kind: ReloadConfig}, {Kind: Backup}}

	case resource.Media:
		if ref(0) == "" {
			return nil
		}
		return []Action{{Kind: DeleteMedia, Target: ref(0)}}

	case resource.Federation:
		if ref(0) == "" {
			return nil
		}
		return []Action{{Kind: PurgeRemoteMedia, Target: ref(0)}}

	case resource.Reports:
		if ref(0) == "" {
			return nil
		}
		return []Action{{Kind: DismissReport, Target: ref(0)}}

	default:
		return nil
	}
}
