// Package live keeps what the TUI knows about the connected phantom-server
// and lays it over the sample listings, which stay in place for everything
// the server cannot tell an unauthenticated client.
package live

import (
	"context"
	"fmt"
	"strings"
	"time"

	tea "charm.land/bubbletea/v2"

	"github.com/HilthonTT/phantom/cli/internal/client"
	"github.com/HilthonTT/phantom/cli/internal/tui/listings"
	"github.com/HilthonTT/phantom/cli/internal/tui/resource"
	"github.com/HilthonTT/phantom/cli/internal/tui/sample"
)

// Interval is how long the TUI waits between probes.
const Interval = 10 * time.Second

type Link int

const (
	Connecting Link = iota
	Connected
	Unreachable
)

type State struct {
	Host string

	Link   Link
	Status client.Status
	Err    error

	Checked time.Time

	Account Account
	Admin   Admin
}

func New(c *client.Client) State { return State{Host: c.Host()} }

// ProbedMsg carries a probe's outcome. Scheduled is set on probes the timer
// started, and only those start the next timer, so a manual refresh never
// adds a second polling loop.
type ProbedMsg struct {
	Status client.Status
	Err    error
	At     time.Time

	Scheduled bool
}

// TickMsg is the timer firing; the app answers it with a scheduled probe.
type TickMsg struct{}

func Probe(c *client.Client, scheduled bool) tea.Cmd {
	return func() tea.Msg {
		status, err := c.Probe(context.Background())
		return ProbedMsg{Status: status, Err: err, At: time.Now(), Scheduled: scheduled}
	}
}

func Tick() tea.Cmd {
	return tea.Tick(Interval, func(time.Time) tea.Msg { return TickMsg{} })
}

func (s State) Apply(msg ProbedMsg) State {
	s.Checked = msg.At
	s.Status, s.Err = msg.Status, msg.Err
	s.Link = Connected
	if msg.Err != nil {
		s.Link = Unreachable
	}

	return s
}

// Differs is whether anything shown outside the connection panel changed;
// latency alone does not, so a steady server never reloads the listings.
func (s State) Differs(o State) bool {
	a, b := s.Status, o.Status
	a.Latency, b.Latency = 0, 0

	return s.Link != o.Link || a != b || errText(s.Err) != errText(o.Err)
}

func errText(err error) string {
	if err == nil {
		return ""
	}

	return err.Error()
}

// Server is the connection panel's view of the link.
func (s State) Server() resource.Server {
	srv := resource.Server{
		Name:  s.Host,
		Admin: s.Account.adminLine(),
	}

	switch s.Link {
	case Connecting:
		srv.State, srv.Status = resource.Running, "connecting"
		srv.Facts = []resource.Field{{Label: "Address", Value: s.Host}}

	case Unreachable:
		srv.State, srv.Status = resource.Failed, "unreachable"
		srv.Facts = []resource.Field{
			{Label: "Address", Value: s.Host},
			{Label: "Error", Value: errText(s.Err), Emphasis: resource.Failed},
			{Label: "Checked", Value: s.Checked.Format(time.TimeOnly)},
		}

	default:
		st := s.Status
		if st.ServerName != "" {
			srv.Name = st.ServerName
		}
		srv.State, srv.Status = resource.Done, "connected"
		srv.Version = st.Software + " " + st.Version
		srv.Facts = []resource.Field{
			{Label: "Address", Value: s.Host},
			{Label: "Latency", Value: latency(st.Latency)},
			{Label: "Federation", Value: onOff(st.Federation)},
		}
	}

	return srv
}

// Listing is the sample listing for section with what the server reported
// written over it.
func (s State) Listing(section resource.Section) resource.Listing {
	if l, ok := s.Admin.listings[section]; ok {
		if err := s.AdminErr(section); err != nil {
			l.Note = "refresh failed: " + err.Error()
		}
		return l
	}

	l := sample.Listing(section)

	switch section {
	case resource.Overview:
		s.overview(&l)
	case resource.API:
		s.api(&l)
	default:
		l.Note = s.sampleNote(section)
	}

	return l
}

// sampleNote says why a section shows sample rows.
func (s State) sampleNote(section resource.Section) string {
	switch {
	case !Served(section):
		return "sample data"
	case !s.Account.Admin:
		return "sample · sign in as an admin"
	case s.AdminErr(section) != nil:
		return "fetch failed: " + s.AdminErr(section).Error()
	default:
		return "loading…"
	}
}

// value is one live overview cell and how it is coloured.
type value struct {
	text  string
	state resource.State
}

func (s State) overview(l *resource.Listing) {
	st := s.Status
	connected := s.Link == Connected

	withheld := func(v string) (string, resource.State) {
		switch {
		case !connected:
			return "unknown, " + s.linkWord(), resource.Held
		case v == "":
			return "withheld, federation is off", resource.Held
		default:
			return v, resource.NoState
		}
	}

	users := ""
	if st.LocalUsers >= 0 {
		users = fmt.Sprint(st.LocalUsers)
	}

	spec, specState := "unknown, "+s.linkWord(), resource.Held
	if connected {
		spec, specState = fmt.Sprintf("%s, %d unstable features", st.Spec, st.Unstable), resource.NoState
	}

	version, versionState := "unknown, "+s.linkWord(), resource.Held
	if connected {
		version, versionState = st.Software+" "+st.Version, resource.NoState
	}

	name, nameState := withheld(st.ServerName)
	usersValue, usersState := withheld(users)

	live := map[string]value{
		"Server name":   {name, nameState},
		"Version":       {version, versionState},
		"Client API":    {spec, specState},
		"HTTP listener": s.listener(),
		"Local users":   {usersValue, usersState},
	}
	if connected && !st.Federation {
		live["Federation"] = value{"off", resource.Held}
	}
	s.adminOverview(live)

	// The client API row is the only one the sample has no stand-in for.
	rows := make([]resource.Row, 0, len(l.Rows)+1)
	for _, r := range l.Rows {
		if unsourced[r.Cells[0]] && s.Admin.Stats != nil {
			continue
		}
		rows = append(rows, r)
		if r.Cells[0] == "Version" {
			rows = append(rows, resource.Row{Cells: []string{"Client API", ""}})
		}
	}

	for i, r := range rows {
		key := r.Cells[0]
		v, ok := live[key]

		source := "sample data"
		if ok {
			source = "phantom-server at " + s.Host
			if adminKeys[key] && s.Admin.Stats != nil {
				source = "phantom-server admin API"
			}
			rows[i] = resource.Row{
				Cells:  []string{key, v.text},
				State:  v.state,
				Marked: r.Marked,
				Detail: []resource.Field{
					{Label: "Property", Value: key},
					{Label: "Value", Value: v.text, Emphasis: v.state},
				},
			}
		}

		rows[i].Detail = append(rows[i].Detail, resource.Field{Label: "Source", Value: source})
	}

	l.Rows = rows
}

// adminKeys are the overview rows the admin API's stats fill.
var adminKeys = map[string]bool{
	"Uptime": true, "Local users": true, "Rooms": true, "Database size": true,
	"Appservices": true, "Registration": true, "Read-only mode": true, "Devices": true,
	"Key backups": true, "Media store": true, "Open reports": true, "Backup": true,
	"Login": true, "Services": true, "Federation": true,
}

// unsourced are overview rows the server keeps nothing to answer, dropped
// once the admin API's figures replace the sample.
var unsourced = map[string]bool{"Events today": true}

// adminOverview fills the overview rows the admin API knows, over the ones
// the unauthenticated probe could.
func (s State) adminOverview(live map[string]value) {
	if d := s.Admin.Devices; d != nil {
		live["Devices"] = value{fmt.Sprint(*d), resource.NoState}
	}

	st := s.Admin.Stats
	if st == nil {
		return
	}

	registration, regState := "closed", resource.NoState
	switch {
	case st.Registration && st.RegistrationToken:
		registration, regState = "open, with a registration token", resource.Held
	case st.Registration:
		registration, regState = "open to anyone", resource.Failed
	}

	readOnly, roState := "off", resource.NoState
	if st.ReadOnly {
		readOnly, roState = "on", resource.Held
	}

	live["Uptime"] = value{listings.Uptime(st.UptimeSecs), resource.NoState}
	live["Local users"] = value{
		fmt.Sprintf("%d, %d of them can sign in", st.LocalUsers, st.ActiveLocalUsers), resource.NoState,
	}
	live["Rooms"] = value{fmt.Sprint(st.Rooms), resource.NoState}
	live["Database size"] = value{listings.Bytes(st.DatabaseBytes), resource.NoState}
	live["Appservices"] = value{fmt.Sprintf("%d registered", st.Appservices), resource.NoState}
	live["Registration"] = value{registration, regState}
	live["Read-only mode"] = value{readOnly, roState}

	live["Key backups"] = value{fmt.Sprintf("%d users", st.KeyBackupUsers), resource.NoState}
	files := "files"
	if st.MediaFiles == 1 {
		files = "file"
	}
	live["Media store"] = value{
		fmt.Sprintf("%s in %d %s", listings.Bytes(st.MediaBytes), st.MediaFiles, files), resource.NoState,
	}

	switch p := s.Admin.Peers; {
	case !st.Federation:
		live["Federation"] = value{"off", resource.Held}
	case p == nil:
	case p.Known == 0:
		live["Federation"] = value{"on; no other server shares a room yet", resource.NoState}
	default:
		state := resource.Done
		if p.Failing > 0 {
			state = resource.Held
		}
		live["Federation"] = value{
			fmt.Sprintf("%d servers, %d reachable, %d backing off", p.Known, p.Reachable, p.Failing), state,
		}
	}

	reports := value{"none", resource.NoState}
	if st.OpenReports > 0 {
		reports = value{fmt.Sprint(st.OpenReports), resource.Held}
	}
	live["Open reports"] = reports

	backup := value{"never; set database_backup_path to turn backups on", resource.Held}
	if st.LastBackupMs != nil {
		at := time.UnixMilli(*st.LastBackupMs)
		backup = value{at.Local().Format("2006-01-02 15:04"), resource.NoState}
		if st.LastBackupBytes != nil {
			backup.text += ", " + listings.Bytes(*st.LastBackupBytes)
		}
		if time.Since(at) > 7*24*time.Hour {
			backup.state = resource.Held
		}
	}
	live["Backup"] = backup

	live["Login"] = value{strings.Join(st.Login, ", "), resource.NoState}

	if c := s.Admin.Services; c != nil {
		state := resource.Done
		if c.Failed > 0 {
			state = resource.Failed
		}
		live["Services"] = value{
			fmt.Sprintf("%d, %d with workers running, %d failed", c.Total, c.Running, c.Failed), state,
		}
	}
}

func (s State) listener() value {
	switch s.Link {
	case Connecting:
		return value{"connecting to " + s.Host, resource.Running}
	case Unreachable:
		return value{"unreachable at " + s.Host + ": " + errText(s.Err), resource.Failed}
	default:
		return value{"serving at " + s.Host + ", " + latency(s.Status.Latency), resource.Done}
	}
}

func (s State) api(l *resource.Listing) {
	for i := range l.Rows {
		r := &l.Rows[i]
		served, state := s.served(r.Cells[0], r.Cells[1])

		for j := range r.Detail {
			if r.Detail[j].Label == "Served" {
				r.Detail[j].Value, r.Detail[j].Emphasis = served, state
			}
		}
	}
}

// served says whether the server answers a module's routes, from the row's
// module name and state word.
func (s State) served(module, word string) (string, resource.State) {
	switch {
	case word == "not ported":
		return "no, not ported", resource.NoState
	case word == "unwired":
		return "no, register() leaves it out", resource.Held
	case s.Link != Connected:
		return "unknown, " + s.linkWord(), resource.Held
	case strings.HasPrefix(module, "federation::") && !s.Status.Federation:
		return "no, federation is off", resource.Held
	default:
		return "yes, at " + s.Host, resource.Done
	}
}

func (s State) linkWord() string {
	if s.Link == Connecting {
		return "still connecting"
	}

	return "server unreachable"
}

func latency(d time.Duration) string {
	if d < time.Millisecond {
		return "<1 ms"
	}

	return fmt.Sprintf("%d ms", d.Milliseconds())
}

func onOff(b bool) string {
	if b {
		return "on"
	}

	return "off"
}
