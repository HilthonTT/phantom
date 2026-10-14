package live

import (
	"context"
	"fmt"
	"strings"
	"time"

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
	BlockServer
	UnblockServer
	QuarantineMedia
	UnquarantineMedia
	PurgeHistory
	PurgeOldMedia
	CancelTask
	ReactivateUser
)

// Action is one thing an admin asked the server to do.
type Action struct {
	Kind ActionKind

	// Target is the user, room, token or device's owner acted on, and Device
	// the device, for signing one out. A room may be named by an alias.
	Target string
	Device string

	// Name is what the listing calls the target, as a room's name, when
	// that is not its ID; typing it confirms as well as the ID does.
	Name string

	// Secret is a new password, or the token to create; empty for a random
	// one.
	Secret string

	// Uses and Days limit a new token; zero leaves it unlimited. Days is
	// also how old history or media must be to be purged.
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
	case BlockServer:
		return "Block"
	case UnblockServer:
		return "Unblock"
	case QuarantineMedia:
		return "Quarantine"
	case UnquarantineMedia:
		return "Lift quarantine"
	case PurgeHistory:
		return "Purge history"
	case PurgeOldMedia:
		return "Purge old media"
	case CancelTask:
		return "Cancel task"
	case ReactivateUser:
		return "Reactivate…"
	default:
		return "Unknown"
	}
}

// NeedsInput is whether the action asks for text before it runs.
func (a Action) NeedsInput() bool {
	return a.Kind == SetPassword || a.Kind == CreateToken || a.Kind == ReactivateUser
}

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
	case BlockServer:
		return "Block " + a.Target + "?",
			"Its requests are refused, nothing is sent to it and its media is not fetched."
	case QuarantineMedia:
		return "Quarantine " + a.Target + "?",
			"It and its thumbnails are answered as missing and never fetched again."
	case PurgeHistory:
		return fmt.Sprintf("Purge %s's history older than %d days?", a.Target, a.Days),
			"Deletes its remote users' messages; state and local users' events are kept."
	case PurgeOldMedia:
		body := "Copies of other servers' files; they are fetched again if asked for."
		if a.Target == "local" {
			body = "This server's own uploads. They exist nowhere else; this cannot be undone."
		}
		return fmt.Sprintf("Purge %s media older than %d days?", a.Target, a.Days), body
	case CancelTask:
		return "Cancel task " + a.Target + "?",
			"It stops at its next safe point; what it has done by then stays done."
	case ReactivateUser:
		return "Reactivate " + a.Target + "?",
			"They can log in with the new password. Rooms, profile and admin rights do not come back."
	default:
		return "", ""
	}
}

// TypeToConfirm is the name to type before the action runs, for those that
// cannot be undone and act on one thing that is easily mistaken for
// another; empty for the rest, which a confirm button is enough for.
func (a Action) TypeToConfirm() string {
	switch a.Kind {
	case DeleteRoom, EraseUser, PurgeRemoteMedia:
		if a.Name != "" {
			return a.Name
		}
		return a.Target
	case PurgeOldMedia:
		// Local uploads are the only copies there are; remote media can be
		// fetched again, so a confirm is enough for that one.
		if a.Target == "local" {
			return a.Target
		}
		return ""
	default:
		return ""
	}
}

// ConfirmedBy says whether what was typed names the action's target: its
// name or its ID, exactly.
func (a Action) ConfirmedBy(typed string) bool {
	typed = strings.TrimSpace(typed)

	return typed != "" && (typed == a.Target || typed == a.Name)
}

// Refreshes are the sections an action's result changes.
func (a Action) Refreshes() []resource.Section {
	switch a.Kind {
	case DeactivateUser, EraseUser, GrantAdmin, RevokeAdmin, ReactivateUser:
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
	case BlockServer, UnblockServer:
		return []resource.Section{resource.Federation, resource.Overview}
	case QuarantineMedia, UnquarantineMedia:
		return []resource.Section{resource.Media}
	case PurgeHistory:
		return []resource.Section{resource.Tasks, resource.Rooms}
	case PurgeOldMedia:
		return []resource.Section{resource.Tasks, resource.Media, resource.Overview}
	case CancelTask:
		return []resource.Section{resource.Tasks, resource.Rooms, resource.Overview}
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

	case ReactivateUser:
		return "Reactivated " + target + ".", c.Reactivate(ctx, target, a.Secret)

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

	case BlockServer:
		return "Blocked " + target + ".", c.BlockServer(ctx, target)

	case UnblockServer:
		still, err := c.UnblockServer(ctx, target)
		if err == nil && len(still) > 0 {
			return target + " is still blocked by the config file: " + strings.Join(still, ", ") + ".", nil
		}
		return "Unblocked " + target + ".", err

	case QuarantineMedia:
		return "Quarantined " + target + ".", c.QuarantineMedia(ctx, target, true)

	case UnquarantineMedia:
		return "Lifted the quarantine of " + target + ".", c.QuarantineMedia(ctx, target, false)

	case PurgeHistory:
		before := time.Now().Add(-time.Duration(a.Days) * 24 * time.Hour).UnixMilli()
		id, err := c.PurgeHistory(ctx, target, before)
		return "Purging " + a.Target + "'s history as task " + id + ".", err

	case PurgeOldMedia:
		id, err := c.PurgeOldMedia(ctx, target, a.Days)
		return "Purging old " + target + " media as task " + id + ".", err
	case CancelTask:
		task, err := c.CancelTask(ctx, target)
		if err == nil && task.Status != "cancelled" {
			return "Asked task " + target + " to stop; it stops at its next safe point.", nil
		}
		return "Cancelled task " + target + ".", err

	default:
		return "", fmt.Errorf("unknown action %d", a.Kind)
	}
}
