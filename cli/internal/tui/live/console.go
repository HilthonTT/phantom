package live

import (
	"context"
	"fmt"
	"strings"
	"sync"
	"time"

	tea "charm.land/bubbletea/v2"

	"github.com/HilthonTT/phantom/cli/internal/client"
	"github.com/HilthonTT/phantom/cli/internal/tui/listings"
	"github.com/HilthonTT/phantom/cli/internal/tui/resource"
)

// The two sections that hold more than one answer: the log, which grows as
// it is followed, and the audit log, which is read a page at a time. Each
// keeps what it was sent and builds its listing from that, so a filter can
// change without asking the server again.

// FollowInterval is how often a followed log asks for its new lines.
const FollowInterval = 2 * time.Second

// AuditPageSize is how many audit records one page asks for.
const AuditPageSize = 200

// LogView is the server's log as the Logs section holds it.
type LogView struct {
	// Lines are newest first, at most LogLines of them.
	Lines []client.LogLine

	Filter listings.LogFilter

	// Follow asks for new lines every FollowInterval and keeps the cursor
	// on the newest; Paused holds a follow without leaving it.
	Follow bool
	Paused bool

	// Debug is whether the server keeps its own debug lines too, once known.
	Debug *bool

	loaded bool
}

// Cursor is the newest line held, for asking for only the lines after it.
func (v LogView) Cursor() int64 {
	if len(v.Lines) == 0 {
		return 0
	}

	return v.Lines[0].Seq
}

// Polling is whether a follow is asking for new lines.
func (v LogView) Polling() bool { return v.Follow && !v.Paused }

func (v LogView) take(lines []client.LogLine, since int64) LogView {
	held := v.Lines
	if since == 0 {
		held = nil
	}

	v.Lines = listings.MergeLogs(held, lines, LogLines)
	v.loaded = true

	return v
}

func (v LogView) listing(now time.Time) resource.Listing {
	kept := listings.FilterLogs(v.Lines, v.Filter)
	l := listings.Logs(kept, now)

	var notes []string
	if v.Filter.Active() {
		notes = append(notes, fmt.Sprintf("%d of %d lines", len(kept), len(v.Lines)), v.Filter.Describe())
	}
	switch {
	case v.Follow && v.Paused:
		notes = append(notes, "paused")
	case v.Follow:
		notes = append(notes, "following")
	}
	if v.Debug != nil && *v.Debug {
		notes = append(notes, "capturing debug")
	}
	if len(notes) > 0 {
		l.Note = strings.Join(notes, " · ")
	}

	return l
}

// AuditView is the audit log as the Audit section holds it.
type AuditView struct {
	// Records are newest first.
	Records []client.AuditRecord

	// Next asks for the page after the last one held; empty once the
	// oldest record is held.
	Next string

	loaded bool
}

func (v AuditView) take(page *client.AuditPage, before string) AuditView {
	if page == nil {
		return v
	}

	// An older page asked for with a cursor the view no longer holds was
	// overtaken by a refresh: appended, it would leave a hole between the
	// fresh first page and itself, so it is dropped and asked for again.
	if before != "" && before != v.Next {
		return v
	}

	if before == "" {
		v.Records = nil
	}
	v.Records = append(append([]client.AuditRecord(nil), v.Records...), page.Records...)
	v.Next = ""
	if page.Next != nil {
		v.Next = *page.Next
	}
	v.loaded = true

	return v
}

func (v AuditView) listing(now time.Time) resource.Listing {
	l := listings.Audit(v.Records, now)
	if v.Next != "" {
		l.Note = fmt.Sprintf("newest %d · o loads older", len(v.Records))
	}

	return l
}

// consoleListing is the listing a section building its own from what it
// holds shows, once it holds anything.
func (s State) consoleListing(section resource.Section) (resource.Listing, bool) {
	now := time.Now()

	switch {
	case section == resource.Logs && s.Admin.Logs.loaded:
		return s.Admin.Logs.listing(now), true
	case section == resource.Audit && s.Admin.Audit.loaded:
		return s.Admin.Audit.listing(now), true
	default:
		return resource.Listing{}, false
	}
}

// WithLogs changes the log view, as a key in the Logs section does.
func (s State) WithLogs(f func(LogView) LogView) State {
	s.Admin.Logs = f(s.Admin.Logs)
	return s
}

// FetchLogs asks for the lines after since, or the whole buffer for zero.
func FetchLogs(c *client.Client, gen int, since int64) tea.Cmd {
	return func() tea.Msg {
		msg := AdminMsg{Gen: gen, Section: resource.Logs, LogsSince: since}
		if since == 0 {
			msg.LogLines, msg.Err = c.Logs(context.Background(), LogLines)
		} else {
			msg.LogLines, msg.Err = c.LogsSince(context.Background(), since, LogLines)
		}

		return msg
	}
}

// FetchOlderAudit asks for the page of the audit log after before.
func FetchOlderAudit(c *client.Client, gen int, before string) tea.Cmd {
	return func() tea.Msg {
		page, err := c.Audit(context.Background(), before, AuditPageSize)
		return AdminMsg{Gen: gen, Section: resource.Audit, AuditPage: &page, AuditBefore: before, Err: err}
	}
}

// LogTickMsg is a followed log's timer firing. Tick numbers each run of the
// timer, so pausing and resuming never leaves two running.
type LogTickMsg struct {
	Gen  int
	Tick int
}

func LogTick(gen, tick int) tea.Cmd {
	return tea.Tick(FollowInterval, func(time.Time) tea.Msg { return LogTickMsg{Gen: gen, Tick: tick} })
}

// LogCaptureMsg is the server's log level, read or just set.
type LogCaptureMsg struct {
	Gen   int
	Debug bool
	Err   error
}

// SetLogCapture asks the server to keep, or stop keeping, its debug lines.
func SetLogCapture(c *client.Client, gen int, debug bool) tea.Cmd {
	return func() tea.Msg {
		level := "info"
		if debug {
			level = "debug"
		}

		ctx := context.Background()
		if err := c.SetLogCapture(ctx, level); err != nil {
			return LogCaptureMsg{Gen: gen, Err: err}
		}

		level, err := c.LogCapture(ctx)
		return LogCaptureMsg{Gen: gen, Debug: level == "debug", Err: err}
	}
}

// TakeLogCapture records the server's log level.
func (s State) TakeLogCapture(msg LogCaptureMsg) State {
	if msg.Gen != s.Admin.Gen || msg.Err != nil {
		return s
	}

	debug := msg.Debug
	s.Admin.Logs.Debug = &debug

	return s
}

// RoomPageSize is how many timeline events the room inspector asks for at
// once.
const RoomPageSize = 50

// RoomMsg is what the room inspector asked for about a room arriving.
type RoomMsg struct {
	RoomID string

	State       []client.StateEvent
	Events      client.TimelinePage
	Extremities []client.Extremity
	Servers     []string

	// Older is set for a further page of the timeline, which is added to
	// the events held rather than replacing everything.
	Older bool

	// From is where an older page was asked to start, so a page that
	// arrives after the inspector was reopened or reloaded is told apart.
	From string

	Err error
}

// FetchRoom asks for everything the inspector shows of a room, all at once.
func FetchRoom(c *client.Client, roomID string) tea.Cmd {
	return func() tea.Msg {
		ctx := context.Background()
		msg := RoomMsg{RoomID: roomID}

		var (
			wg   sync.WaitGroup
			mu   sync.Mutex
			errs []error
		)
		fail := func(err error) {
			if err != nil {
				mu.Lock()
				errs = append(errs, err)
				mu.Unlock()
			}
		}

		wg.Go(func() {
			var err error
			msg.State, err = c.RoomState(ctx, roomID)
			fail(err)
		})
		wg.Go(func() {
			var err error
			msg.Events, err = c.RoomEvents(ctx, roomID, "", RoomPageSize)
			fail(err)
		})
		wg.Go(func() {
			var err error
			msg.Extremities, err = c.RoomExtremities(ctx, roomID)
			fail(err)
		})
		wg.Go(func() {
			var err error
			msg.Servers, err = c.RoomServers(ctx, roomID)
			fail(err)
		})
		wg.Wait()

		if len(errs) > 0 {
			msg.Err = errs[0]
		}

		return msg
	}
}

// FetchOlderEvents asks for the page of a room's timeline after from.
func FetchOlderEvents(c *client.Client, roomID, from string) tea.Cmd {
	return func() tea.Msg {
		page, err := c.RoomEvents(context.Background(), roomID, from, RoomPageSize)
		return RoomMsg{RoomID: roomID, Events: page, Older: true, From: from, Err: err}
	}
}
