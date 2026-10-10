package live

import (
	"context"
	"fmt"
	"strings"

	tea "charm.land/bubbletea/v2"

	"github.com/HilthonTT/phantom/cli/internal/client"
	"github.com/HilthonTT/phantom/cli/internal/tui/resource"
)

type ActionKind int

const (
	DeactivateUser ActionKind = iota
	EraseUser
	SetPassword
	GrantAdmin
	RevokeAdmin
	SignOutDevice
	CreateToken
	RevokeToken
	BanRoom
	UnbanRoom
	ShutdownRoom
	DeleteRoom
	ReloadConfig
	Backup
	DeleteMedia
	PurgeRemoteMedia
	DismissReport
)

// Action is one thing an admin asked the server to do.
type Action struct {
	Kind ActionKind

	// Target is the user, room, token or device's owner acted on, and Device
	// the device, for signing one out. A room may be named by an alias.
	Target string
	Device string

	// Secret is a new password, or the token to create; empty for a random
	// one.
	Secret string

	// Uses and Days limit a new token; zero leaves it unlimited.
	Uses int64
	Days int64
}

// Label is the action as a menu or prompt names it.
func (a Action) Label() string {
	switch a.Kind {
	case DeactivateUser:
		return "Deactivate"
	case EraseUser:
		return "Deactivate and erase"
	case SetPassword:
		return "Set password…"
	case GrantAdmin:
		return "Make admin"
	case RevokeAdmin:
		return "Revoke admin"
	case SignOutDevice:
		return "Sign out"
	case CreateToken:
		return "New token…"
	case RevokeToken:
		return "Revoke"
	case BanRoom:
		return "Ban"
	case UnbanRoom:
		return "Unban"
	case ShutdownRoom:
		return "Shut down"
	case DeleteRoom:
		return "Delete and purge"
	case ReloadConfig:
		return "Reload config"
	case Backup:
		return "Back up database"
	case DeleteMedia:
		return "Delete"
	case PurgeRemoteMedia:
		return "Purge its media"
	case DismissReport:
		return "Dismiss"
	default:
		return "Unknown"
	}
}

// NeedsInput is whether the action asks for text before it runs.
func (a Action) NeedsInput() bool { return a.Kind == SetPassword || a.Kind == CreateToken }

// Confirm is what to ask before running the action; empty runs it at once.
func (a Action) Confirm() (title, body string) {
	switch a.Kind {
	case DeactivateUser:
		return "Deactivate " + a.Target + "?",
			"Signs out every device, clears the password and leaves every room."
	case EraseUser:
		return "Deactivate and erase " + a.Target + "?",
			"As deactivating, and also wipes the account's data. This cannot be undone."
	case SetPassword:
		return "Set " + a.Target + "'s password?", "Their devices are signed out."
	case GrantAdmin:
		return "Make " + a.Target + " an admin?", "They join the admin room and can manage the server."
	case RevokeAdmin:
		return "Revoke " + a.Target + "'s admin?", "They are removed from the admin room."
	case SignOutDevice:
		return "Sign out " + a.Device + "?", "Ends the session of " + a.Target + " on that device."
	case RevokeToken:
		return "Revoke " + a.Target + "?", "Registration stops accepting it."
	case BanRoom:
		return "Ban " + a.Target + "?", "Local users can no longer join it."
	case ShutdownRoom:
		return "Shut down " + a.Target + "?", "Evicts every local member and frees its aliases."
	case DeleteRoom:
		return "Delete " + a.Target + "?",
			"Shuts it down, then purges all the server holds of it. This cannot be undone."
	case DeleteMedia:
		return "Delete " + a.Target + "?", "Removes the file and its thumbnails. This cannot be undone."
	case PurgeRemoteMedia:
		return "Purge " + a.Target + "'s media?",
			"Deletes every copy of its files held here; they are fetched again if asked for."
	case DismissReport:
		return "Dismiss this report?", "It is closed and leaves the list."
	default:
		return "", ""
	}
}

// Refreshes are the sections an action's result changes.
func (a Action) Refreshes() []resource.Section {
	switch a.Kind {
	case DeactivateUser, EraseUser, GrantAdmin, RevokeAdmin:
		return []resource.Section{resource.Users, resource.Devices, resource.Overview}
	case SetPassword, SignOutDevice:
		return []resource.Section{resource.Devices, resource.Users}
	case CreateToken, RevokeToken:
		return []resource.Section{resource.Tokens}
	case BanRoom, UnbanRoom, ShutdownRoom, DeleteRoom:
		return []resource.Section{resource.Rooms, resource.Overview, resource.Tasks}
	case ReloadConfig:
		return []resource.Section{resource.Settings, resource.Overview}
	case Backup:
		return []resource.Section{resource.Tasks, resource.Overview}
	case DeleteMedia, PurgeRemoteMedia:
		return []resource.Section{resource.Media, resource.Overview}
	case DismissReport:
		return []resource.Section{resource.Reports, resource.Overview}
	default:
		return nil
	}
}

// ActedMsg is an action finishing.
type ActedMsg struct {
	Action Action

	// Done says what happened, for the notice.
	Done string
	Err  error
}

// Act runs an action against the admin API.
func Act(c *client.Client, a Action) tea.Cmd {
	return func() tea.Msg {
		done, err := act(context.Background(), c, a)
		return ActedMsg{Action: a, Done: done, Err: err}
	}
}

func act(ctx context.Context, c *client.Client, a Action) (string, error) {
	target := a.Target
	if strings.HasPrefix(target, "#") {
		id, err := c.ResolveAlias(ctx, target)
		if err != nil {
			return "", fmt.Errorf("%s: %w", target, err)
		}
		target = id
	}

	switch a.Kind {
	case DeactivateUser, EraseUser:
		return "Deactivated " + target + ".", c.Deactivate(ctx, target, a.Kind == EraseUser)

	case SetPassword:
		return "Set " + target + "'s password and signed out their devices.",
			c.SetPassword(ctx, target, a.Secret, true)

	case GrantAdmin:
		return target + " is now an admin.", c.GrantAdmin(ctx, target)

	case RevokeAdmin:
		return target + " is no longer an admin.", c.RevokeAdmin(ctx, target)

	case SignOutDevice:
		return "Signed out " + a.Device + ".", c.DeleteDevice(ctx, target, a.Device)

	case CreateToken:
		req := client.NewToken{}
		if a.Secret != "" {
			req.Token = &a.Secret
		}
		if a.Uses > 0 {
			req.UsesAllowed = &a.Uses
		}
		if a.Days > 0 {
			secs := a.Days * 24 * 60 * 60
			req.ExpiresInSecs = &secs
		}
		tok, err := c.CreateToken(ctx, req)
		return "Created token " + tok.Token + ".", err

	case RevokeToken:
		return "Revoked " + target + ".", c.RevokeToken(ctx, target)

	case BanRoom:
		return "Banned " + a.Target + ".", c.BanRoom(ctx, target, true)

	case UnbanRoom:
		return "Unbanned " + a.Target + ".", c.BanRoom(ctx, target, false)

	case ShutdownRoom:
		id, err := c.ShutdownRoom(ctx, target)
		return "Shutting " + a.Target + " down as task " + id + ".", err

	case DeleteRoom:
		id, err := c.DeleteRoom(ctx, target, false)
		return "Deleting " + a.Target + " as task " + id + ".", err

	case ReloadConfig:
		return "Reloaded the config.", c.ReloadConfig(ctx)

	case Backup:
		id, err := c.Backup(ctx)
		return "Backing up the database as task " + id + ".", err

	case DeleteMedia:
		return "Deleted " + target + ".", c.DeleteMedia(ctx, target)

	case PurgeRemoteMedia:
		n, err := c.PurgeRemoteMedia(ctx, target)
		return fmt.Sprintf("Deleted %d files of %s.", n, target), err

	case DismissReport:
		return "Dismissed the report.", c.DismissReport(ctx, target)

	default:
		return "", fmt.Errorf("unknown action %d", a.Kind)
	}
}
