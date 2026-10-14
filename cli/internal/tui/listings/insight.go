package listings

import (
	"cmp"
	"fmt"
	"slices"
	"strings"
	"time"

	"github.com/HilthonTT/phantom/cli/internal/client"
	"github.com/HilthonTT/phantom/cli/internal/tui/resource"
	"github.com/HilthonTT/phantom/cli/internal/tui/sample"
)

// Services lists the server's services by name, with their workers' state.
func Services(services []client.AdminService, now time.Time) resource.Listing {
	rows := make([]resource.Row, 0, len(services))
	for _, svc := range services {
		state, word := resource.NoState, svc.Status
		switch svc.Status {
		case "running":
			state = resource.Running
		case "finished":
			// A service with no background work returns at once.
			state, word = resource.Done, "ready"
		case "failed":
			state = resource.Failed
		}

		purpose := sample.Purpose(svc.Name)
		if purpose == "" {
			purpose = "—"
		}

		rows = append(rows, resource.Row{
			Cells: []string{svc.Name, word, purpose},
			State: state,
			Detail: []resource.Field{
				{Label: "Service", Value: svc.Name},
				{Label: "State", Value: word, Emphasis: state},
				{Label: "Purpose", Value: purpose},
				{Label: "Started", Value: when(svc.StartedAtMs, now)},
				{Label: "Stopped", Value: when(svc.StoppedAtMs, now)},
				{Label: "Restarts", Value: fmt.Sprint(svc.Restarts)},
				{Label: "Error", Value: or(svc.Error, "—"), Emphasis: flag(svc.Error != nil, resource.Failed)},
				{Label: "Source", Value: Source},
			},
		})
	}

	return resource.Listing{
		Sort: "name",
		Columns: []resource.Column{
			{Title: "Service", Width: 26},
			{Title: "State", Width: 9},
			{Title: "Purpose", Flex: true},
		},
		Rows: rows,
	}
}

// ServiceCounts reads a services listing back into its tallies, for the
// overview: how many there are, and how many have a worker running.
func ServiceCounts(services []client.AdminService) (total, running, failed int) {
	for _, svc := range services {
		total++
		switch svc.Status {
		case "running":
			running++
		case "failed":
			failed++
		}
	}

	return total, running, failed
}

// PeerCounts tallies the servers this one shares rooms with, for the
// overview: all of them, those that answered since startup, and those held
// back after failures.
func PeerCounts(peers []client.Peer, now time.Time) (known, reachable, failing int) {
	for _, p := range peers {
		known++
		switch {
		case held(p, now):
			failing++
		case p.Backoff == nil && p.LastContactMs != nil:
			reachable++
		}
	}

	return known, reachable, failing
}

// retryAt is when the server next tries a failing peer: its backoff counts
// from the last failure, not from now.
func retryAt(p client.Peer) time.Time {
	return time.UnixMilli(p.Backoff.SinceMs).Add(time.Duration(p.Backoff.DelaySecs) * time.Second)
}

// held says whether requests to the peer are being held back now. Once its
// backoff has run out the server tries it again, just after other peers.
func held(p client.Peer, now time.Time) bool {
	return p.Backoff != nil && (p.Backoff.Permanent || retryAt(p).After(now))
}

// failingAfter is how many failures in a row turn a server answering now
// and then from flaky to failing.
const failingAfter = 5

// PeerHealth is how talking to a server is going, in a word and a colour:
// held back or failing in red and amber, answering in green.
func PeerHealth(p client.Peer, now time.Time) (string, resource.State) {
	switch {
	case p.Backoff != nil && p.Backoff.Permanent:
		return "unreachable", resource.Failed
	case held(p, now):
		return "backing off", resource.Held
	case p.Backoff != nil:
		return "retrying", resource.Held
	case p.ConsecutiveFailures >= failingAfter:
		return "failing", resource.Failed
	case p.ConsecutiveFailures > 0:
		return "flaky", resource.Held
	case p.LastContactMs != nil:
		return "reachable", resource.Done
	case p.LastInboundMs != nil:
		return "inbound only", resource.Done
	default:
		return "not contacted", resource.NoState
	}
}

// peerRank sorts servers held back first, then those failing, then the
// rest.
func peerRank(p client.Peer) int {
	switch {
	case p.Backoff != nil:
		return 0
	case p.ConsecutiveFailures > 0:
		return 1
	default:
		return 2
	}
}

// Federation lists the servers this one shares rooms with, those held back
// after failures first, then those failing, then by last contact.
func Federation(peers []client.Peer, now time.Time) resource.Listing {
	slices.SortStableFunc(peers, func(a, b client.Peer) int {
		if c := cmp.Compare(peerRank(a), peerRank(b)); c != 0 {
			return c
		}
		return cmp.Compare(ms(b.LastContactMs), ms(a.LastContactMs))
	})

	rows := make([]resource.Row, 0, len(peers))
	for _, p := range peers {
		status, state := PeerHealth(p, now)

		backoff := "—"
		if p.Backoff != nil {
			last := time.UnixMilli(p.Backoff.SinceMs).In(now.Location()).Format("2006-01-02 15:04")
			switch {
			case p.Backoff.Permanent:
				backoff = "given up; last failure " + last
			case held(p, now):
				backoff = fmt.Sprintf("retry in %s; last failure %s", retryAt(p).Sub(now).Round(time.Second), last)
			default:
				backoff = "retrying, after other servers; last failure " + last
			}
		}

		blocked := ""
		if p.Blocked {
			blocked = "blocked"
		}

		lastError := or(p.LastError, "—")
		if p.LastFailureMs != nil {
			lastError = ago(p.LastFailureMs, now) + ": " + lastError
		}

		rows = append(rows, resource.Row{
			Cells:    []string{p.Server, status, fmt.Sprint(p.Queued), fmt.Sprint(p.Rooms), ago(p.LastContactMs, now), yesNo(p.Blocked)},
			Ref:      []string{p.Server, blocked},
			SortKeys: map[int]string{2: fmt.Sprintf("%09d", p.Queued), 3: fmt.Sprintf("%09d", p.Rooms), 4: fmt.Sprint(ms(p.LastContactMs))},
			State:    state,
			Detail: []resource.Field{
				{Label: "Server", Value: p.Server},
				{Label: "Status", Value: status, Emphasis: state},
				{Label: "Shared rooms", Value: fmt.Sprint(p.Rooms)},
				{Label: "Last contact", Value: when(p.LastContactMs, now) + " (since startup)"},
				{Label: "Backoff", Value: backoff, Emphasis: flag(p.Backoff != nil, state)},
				{Label: "Resolved to", Value: or(p.Resolved, "not cached")},
				{Label: "Sending", Value: fmt.Sprintf("%d in flight", p.Sending)},
				{Label: "Queued", Value: fmt.Sprintf("%d waiting", p.Queued), Emphasis: flag(p.Queued > 0, resource.Held)},
				{Label: "Last error", Value: lastError, Emphasis: flag(p.LastError != nil, state)},
				{Label: "Failing for", Value: failures(p.ConsecutiveFailures), Emphasis: flag(p.ConsecutiveFailures > 0, state)},
				{Label: "Last inbound", Value: when(p.LastInboundMs, now) + " (since startup)"},
				{Label: "Blocked", Value: yesNo(p.Blocked), Emphasis: flag(p.Blocked, resource.Failed)},
				{Label: "Source", Value: Source},
			},
		})
	}

	return resource.Listing{
		Sort: "failing first, then last contact",
		Columns: []resource.Column{
			{Title: "Server", Flex: true},
			{Title: "Health", Width: 13},
			{Title: "Queue", Width: 6, Right: true},
			{Title: "Rooms", Width: 7, Right: true},
			{Title: "Last contact", Width: 14, Right: true},
			{Title: "Blocked", Width: 8},
		},
		Rows: rows,
	}
}

func failures(n int) string {
	switch n {
	case 0:
		return "—"
	case 1:
		return "1 request in a row"
	default:
		return fmt.Sprintf("%d requests in a row", n)
	}
}

// Media lists the stored files, largest first.
func Media(media []client.StoredMedia, now time.Time) resource.Listing {
	slices.SortStableFunc(media, func(a, b client.StoredMedia) int { return cmp.Compare(b.Size, a.Size) })

	rows := make([]resource.Row, 0, len(media))
	for _, m := range media {
		id := m.MXC
		if _, rest, ok := strings.Cut(strings.TrimPrefix(m.MXC, "mxc://"), "/"); ok {
			id = rest
		}

		origin := "local upload"
		if !m.Local {
			origin = "remote copy"
		}

		quarantined, state := "", resource.NoState
		if m.Quarantined {
			quarantined, state = "quarantined", resource.Held
		}

		created := m.CreatedAtMs
		rows = append(rows, resource.Row{
			Cells:    []string{id, Bytes(m.Size), or(m.ContentType, "—"), localpartOr(m.Uploader), yesNo(m.Quarantined)},
			Ref:      []string{m.MXC, quarantined},
			State:    state,
			SortKeys: map[int]string{1: fmt.Sprint(m.Size)},
			Detail: []resource.Field{
				{Label: "Media", Value: m.MXC},
				{Label: "Size", Value: Bytes(m.Size)},
				{Label: "Type", Value: or(m.ContentType, "unknown")},
				{Label: "Uploader", Value: or(m.Uploader, "—")},
				{Label: "Stored", Value: when(&created, now)},
				{Label: "Origin", Value: origin},
				{Label: "Thumbnails", Value: fmt.Sprint(m.Thumbnails)},
				{Label: "Quarantined", Value: yesNo(m.Quarantined), Emphasis: flag(m.Quarantined, resource.Held)},
				{Label: "Source", Value: Source},
			},
		})
	}

	return resource.Listing{
		Sort: "size, largest first",
		Columns: []resource.Column{
			{Title: "Media ID", Flex: true},
			{Title: "Size", Width: 10, Right: true},
			{Title: "Type", Width: 18},
			{Title: "Uploader", Width: 12},
			{Title: "Quarantined", Width: 12},
		},
		Rows: rows,
	}
}

// Logs lists the server's recent lines as it sent them, newest first.
func Logs(lines []client.LogLine, now time.Time) resource.Listing {
	rows := make([]resource.Row, 0, len(lines))
	for _, l := range lines {
		state := resource.NoState
		switch l.Level {
		case "ERROR":
			state = resource.Failed
		case "WARN":
			state = resource.Held
		case "DEBUG", "TRACE":
			state = resource.Running
		}

		at := l.AtMs
		stamp := time.UnixMilli(at).In(now.Location())
		target := strings.TrimPrefix(l.Target, "phantom_")

		rows = append(rows, resource.Row{
			Cells:    []string{stamp.Format("15:04:05"), l.Level, target, l.Message},
			Ref:      []string{fmt.Sprint(l.Seq)},
			SortKeys: map[int]string{0: fmt.Sprint(at)},
			State:    state,
			Detail: []resource.Field{
				{Label: "Time", Value: stamp.Format("2006-01-02 15:04:05.000")},
				{Label: "Level", Value: l.Level, Emphasis: state},
				{Label: "Target", Value: l.Target},
				{Label: "Span", Value: orEmpty(l.Span, "—")},
				{Label: "Message", Value: l.Message},
				{Label: "Source", Value: Source},
			},
		})
	}

	return resource.Listing{
		Sort: "newest first",
		Columns: []resource.Column{
			{Title: "Time", Width: 10},
			{Title: "Level", Width: 7},
			{Title: "Target", Width: 24},
			{Title: "Message", Flex: true},
		},
		Rows: rows,
	}
}

// Reports lists the open abuse reports as the server sent them, newest
// first.
func Reports(reports []client.Report, now time.Time) resource.Listing {
	rows := make([]resource.Row, 0, len(reports))
	for _, r := range reports {
		target := or(r.UserID, "—")
		switch r.Kind {
		case "event":
			target = or(r.EventID, "—")
		case "room":
			target = or(r.RoomID, "—")
		}

		at := r.AtMs
		rows = append(rows, resource.Row{
			Cells:    []string{time.UnixMilli(at).In(now.Location()).Format("01-02 15:04"), r.Kind, target, r.Reason},
			Ref:      []string{r.ID},
			SortKeys: map[int]string{0: fmt.Sprint(at)},
			State:    resource.Held,
			Detail: []resource.Field{
				{Label: "Kind", Value: r.Kind},
				{Label: "Target", Value: target},
				{Label: "Room", Value: or(r.RoomID, "—")},
				{Label: "Sent by", Value: or(r.UserID, "—")},
				{Label: "Reported by", Value: r.Reporter},
				{Label: "Received", Value: when(&at, now)},
				{Label: "Reason", Value: orEmpty(r.Reason, "none given")},
				{Label: "Source", Value: Source},
			},
		})
	}

	return resource.Listing{
		Sort: "newest first",
		Columns: []resource.Column{
			{Title: "Time", Width: 12},
			{Title: "Kind", Width: 7},
			{Title: "Target", Width: 28},
			{Title: "Reason", Flex: true},
		},
		Rows: rows,
	}
}

func orEmpty(s, fallback string) string {
	if s == "" {
		return fallback
	}

	return s
}

func localpartOr(id *string) string {
	if id == nil {
		return "—"
	}

	return localpart(*id)
}
