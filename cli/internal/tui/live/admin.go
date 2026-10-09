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

	listings map[resource.Section]resource.Listing
	errs     map[resource.Section]error
}

// AdminMsg is one section's answer arriving.
type AdminMsg struct {
	Gen     int
	Section resource.Section

	Listing resource.Listing
	Stats   *client.Stats
	Devices *int

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
