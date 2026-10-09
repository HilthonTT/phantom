// Package listings draws the admin API's answers as the workspace's tables,
// one per section, in the columns the sample data set out.
package listings

import (
	"cmp"
	"fmt"
	"slices"
	"strings"
	"time"

	"github.com/HilthonTT/phantom/cli/internal/client"
	"github.com/HilthonTT/phantom/cli/internal/tui/resource"
)

// Source is what every live row's detail names as its origin.
const Source = "phantom-server admin API"

func Users(users []client.AdminUser, now time.Time) resource.Listing {
	slices.SortStableFunc(users, func(a, b client.AdminUser) int {
		return cmp.Compare(ms(b.LastSeenMs), ms(a.LastSeenMs))
	})

	rows := make([]resource.Row, 0, len(users))
	for _, u := range users {
		state, emphasis := "active", resource.Done
		switch {
		case u.ServerUser:
			state, emphasis = "server user", resource.Held
		case u.Deactivated:
			state, emphasis = "deactivated", resource.Failed
		}

		seen := ago(u.LastSeenMs, now)

		rows = append(rows, resource.Row{
			Cells: []string{u.UserID, yesNo(u.Admin), state, seen},
			State: emphasis,
			Detail: []resource.Field{
				{Label: "User ID", Value: u.UserID},
				{Label: "Display name", Value: or(u.DisplayName, "—")},
				{Label: "Admin", Value: yesNo(u.Admin)},
				{Label: "State", Value: state, Emphasis: emphasis},
				{Label: "Last seen", Value: when(u.LastSeenMs, now)},
				{Label: "Devices", Value: fmt.Sprint(u.Devices)},
				{Label: "Rooms joined", Value: fmt.Sprint(u.RoomsJoined)},
				{Label: "Source", Value: Source},
			},
		})
	}

	return resource.Listing{
		Sort: "last seen, newest first",
		Columns: []resource.Column{
			{Title: "User", Flex: true},
			{Title: "Admin", Width: 7},
			{Title: "State", Width: 12},
			{Title: "Last seen", Width: 14, Right: true},
		},
		Rows: rows,
	}
}

func Devices(devices []client.AdminDevice, now time.Time) resource.Listing {
	slices.SortStableFunc(devices, func(a, b client.AdminDevice) int {
		return cmp.Compare(ms(b.LastSeenMs), ms(a.LastSeenMs))
	})

	rows := make([]resource.Row, 0, len(devices))
	for _, d := range devices {
		rows = append(rows, resource.Row{
			Cells: []string{d.DeviceID, localpart(d.UserID), or(d.DisplayName, "—"), or(d.LastSeenIP, "—"), ago(d.LastSeenMs, now)},
			Detail: []resource.Field{
				{Label: "Device ID", Value: d.DeviceID},
				{Label: "User", Value: d.UserID},
				{Label: "Display name", Value: or(d.DisplayName, "—")},
				{Label: "Last seen", Value: when(d.LastSeenMs, now)},
				{Label: "Last IP", Value: or(d.LastSeenIP, "—")},
				{Label: "Source", Value: Source},
			},
		})
	}

	return resource.Listing{
		Sort: "last seen, newest first",
		Columns: []resource.Column{
			{Title: "Device", Width: 12},
			{Title: "User", Width: 10},
			{Title: "Name", Flex: true},
			{Title: "Last IP", Width: 16},
			{Title: "Last seen", Width: 12, Right: true},
		},
		Rows: rows,
	}
}

func Tokens(tokens []client.RegistrationToken, now time.Time) resource.Listing {
	// Soonest expiry first; tokens that never expire last.
	slices.SortStableFunc(tokens, func(a, b client.RegistrationToken) int {
		switch {
		case a.ExpiresAtMs == nil && b.ExpiresAtMs == nil:
			return 0
		case a.ExpiresAtMs == nil:
			return 1
		case b.ExpiresAtMs == nil:
			return -1
		}
		return cmp.Compare(*a.ExpiresAtMs, *b.ExpiresAtMs)
	})

	rows := make([]resource.Row, 0, len(tokens))
	for _, tok := range tokens {
		uses, limit, expires := "—", "none", "never"
		if tok.Uses != nil {
			uses = fmt.Sprint(*tok.Uses)
		}
		if tok.MaxUses != nil {
			limit = fmt.Sprintf("%d uses", *tok.MaxUses)
		}
		if tok.ExpiresAtMs != nil {
			expires = time.UnixMilli(*tok.ExpiresAtMs).In(now.Location()).Format("2006-01-02 15:04")
		}

		// A token one use or a day from running out is flagged.
		state := resource.Done
		if (tok.MaxUses != nil && tok.Uses != nil && *tok.MaxUses-*tok.Uses <= 1) ||
			(tok.ExpiresAtMs != nil && time.UnixMilli(*tok.ExpiresAtMs).Sub(now) < 24*time.Hour) {
			state = resource.Held
		}

		source := "database"
		if tok.Source == "config" {
			source = "config file (the token is masked)"
		}

		rows = append(rows, resource.Row{
			Cells: []string{tok.Token, uses, limit, expires},
			State: state,
			Detail: []resource.Field{
				{Label: "Token", Value: tok.Token},
				{Label: "Uses", Value: uses},
				{Label: "Max uses", Value: limit},
				{Label: "Expires", Value: expires, Emphasis: state},
				{Label: "Stored in", Value: source},
				{Label: "Source", Value: Source},
			},
		})
	}

	return resource.Listing{
		Sort: "expiry, soonest first",
		Columns: []resource.Column{
			{Title: "Token", Flex: true},
			{Title: "Uses", Width: 6, Right: true},
			{Title: "Max uses", Width: 10, Right: true},
			{Title: "Expires", Width: 20, Right: true},
		},
		Rows: rows,
	}
}

func Rooms(rooms []client.AdminRoom) resource.Listing {
	slices.SortStableFunc(rooms, func(a, b client.AdminRoom) int {
		return cmp.Compare(b.JoinedMembers, a.JoinedMembers)
	})

	rows := make([]resource.Row, 0, len(rooms))
	for _, r := range rooms {
		title := or(r.Name, or(r.CanonicalAlias, r.RoomID))

		visibility := r.JoinRule
		if r.Published {
			visibility += ", listed"
		}

		encryption, encrypted := "no", resource.Held
		if r.Encrypted {
			encryption, encrypted = "yes", resource.Done
		}

		state := resource.NoState
		switch {
		case r.Banned:
			state = resource.Failed
		case r.Disabled:
			state = resource.Held
		}

		rows = append(rows, resource.Row{
			Cells: []string{title, fmt.Sprint(r.JoinedMembers), or(r.Version, "—"), visibility},
			State: state,
			Detail: []resource.Field{
				{Label: "Name", Value: or(r.Name, "—")},
				{Label: "Alias", Value: or(r.CanonicalAlias, "—")},
				{Label: "Room ID", Value: r.RoomID},
				{Label: "Topic", Value: or(r.Topic, "—")},
				{Label: "Members", Value: fmt.Sprintf("%d, %d of them local", r.JoinedMembers, r.LocalMembers)},
				{Label: "Version", Value: or(r.Version, "unknown")},
				{Label: "Join rule", Value: r.JoinRule},
				{Label: "In directory", Value: yesNo(r.Published)},
				{Label: "Encrypted", Value: encryption, Emphasis: encrypted},
				{Label: "Banned", Value: yesNo(r.Banned), Emphasis: flag(r.Banned, resource.Failed)},
				{Label: "Disabled", Value: yesNo(r.Disabled), Emphasis: flag(r.Disabled, resource.Held)},
				{Label: "Source", Value: Source},
			},
		})
	}

	return resource.Listing{
		Sort: "members, descending",
		Columns: []resource.Column{
			{Title: "Room", Flex: true},
			{Title: "Members", Width: 9, Right: true},
			{Title: "Ver", Width: 5, Right: true},
			{Title: "Visibility", Width: 16},
		},
		Rows: rows,
	}
}

func Appservices(appservices []client.Appservice) resource.Listing {
	slices.SortStableFunc(appservices, func(a, b client.Appservice) int { return strings.Compare(a.ID, b.ID) })

	rows := make([]resource.Row, 0, len(appservices))
	for _, as := range appservices {
		rows = append(rows, resource.Row{
			Cells: []string{as.ID, list(as.Users), yesNo(as.RateLimited)},
			Detail: []resource.Field{
				{Label: "Registration", Value: as.ID},
				{Label: "URL", Value: or(as.URL, "none; push disabled")},
				{Label: "Sender", Value: "@" + as.SenderLocalpart},
				{Label: "Users", Value: list(as.Users)},
				{Label: "Aliases", Value: list(as.Aliases)},
				{Label: "Rooms", Value: list(as.Rooms)},
				{Label: "Protocols", Value: list(as.Protocols)},
				{Label: "Rate limited", Value: yesNo(as.RateLimited)},
				{Label: "Source", Value: Source},
			},
		})
	}

	return resource.Listing{
		Sort: "registration ID",
		Columns: []resource.Column{
			{Title: "Registration", Flex: true},
			{Title: "Users", Width: 18},
			{Title: "Rate limited", Width: 13},
		},
		Rows: rows,
	}
}

func Settings(settings []client.Setting) resource.Listing {
	slices.SortStableFunc(settings, func(a, b client.Setting) int { return strings.Compare(a.Key, b.Key) })

	rows := make([]resource.Row, 0, len(settings))
	for _, s := range settings {
		value := unquote(s.Value)

		rows = append(rows, resource.Row{
			Cells: []string{s.Key, value},
			Detail: []resource.Field{
				{Label: "Key", Value: s.Key},
				{Label: "Value", Value: value},
				{Label: "Default", Value: "see phantom-example.toml"},
				{Label: "Source", Value: Source},
			},
		})
	}

	return resource.Listing{
		Sort: "key",
		Columns: []resource.Column{
			{Title: "Key", Width: 36},
			{Title: "Value", Flex: true},
		},
		Rows: rows,
	}
}

// unquote drops the quotes the server's debug printing puts around a plain
// string; anything else is left as printed.
func unquote(value string) string {
	if len(value) >= 2 && strings.HasPrefix(value, `"`) && strings.HasSuffix(value, `"`) &&
		!strings.Contains(value[1:len(value)-1], `"`) {
		return value[1 : len(value)-1]
	}

	return value
}

// Uptime reads like "6d 04:11", or "04:11" under a day.
func Uptime(secs int64) string {
	d := time.Duration(secs) * time.Second
	days := int(d.Hours()) / 24
	hm := fmt.Sprintf("%02d:%02d", int(d.Hours())%24, int(d.Minutes())%60)

	if days > 0 {
		return fmt.Sprintf("%dd %s", days, hm)
	}

	return hm
}

// Bytes reads like "1.8 GiB".
func Bytes(n int64) string {
	const unit = 1024

	if n < unit {
		return fmt.Sprintf("%d B", n)
	}

	div, exp := int64(unit), 0
	for m := n / unit; m >= unit; m /= unit {
		div *= unit
		exp++
	}

	return fmt.Sprintf("%.1f %ciB", float64(n)/float64(div), "KMGTPE"[exp])
}

// ago reads like "2 min ago".
func ago(at *int64, now time.Time) string {
	if at == nil {
		return "never"
	}

	d := now.Sub(time.UnixMilli(*at))
	switch {
	case d < time.Minute:
		return "just now"
	case d < time.Hour:
		return fmt.Sprintf("%d min ago", int(d.Minutes()))
	case d < 48*time.Hour:
		return fmt.Sprintf("%d hours ago", int(d.Hours()))
	default:
		return fmt.Sprintf("%d days ago", int(d.Hours()/24))
	}
}

// when is a time in full, with how long ago it was.
func when(at *int64, now time.Time) string {
	if at == nil {
		return "never"
	}

	return time.UnixMilli(*at).In(now.Location()).Format("2006-01-02 15:04") + ", " + ago(at, now)
}

func ms(at *int64) int64 {
	if at == nil {
		return 0
	}

	return *at
}

func or(s *string, fallback string) string {
	if s == nil || *s == "" {
		return fallback
	}

	return *s
}

func list(items []string) string {
	if len(items) == 0 {
		return "—"
	}

	return strings.Join(items, ", ")
}

func yesNo(b bool) string {
	if b {
		return "yes"
	}

	return "no"
}

func flag(set bool, state resource.State) resource.State {
	if set {
		return state
	}

	return resource.NoState
}

func localpart(id string) string {
	id = strings.TrimPrefix(id, "@")
	if name, _, ok := strings.Cut(id, ":"); ok {
		return "@" + name
	}

	return id
}
