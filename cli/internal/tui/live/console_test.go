package live

import (
	"strings"
	"testing"

	"github.com/HilthonTT/phantom/cli/internal/client"
	"github.com/HilthonTT/phantom/cli/internal/tui/listings"
	"github.com/HilthonTT/phantom/cli/internal/tui/resource"
)

func logLines(seqs ...int64) []client.LogLine {
	lines := make([]client.LogLine, 0, len(seqs))
	for _, seq := range seqs {
		lines = append(lines, client.LogLine{Seq: seq, Level: "INFO", Message: "line"})
	}

	return lines
}

func TestAFollowedLogGrowsFromItsCursor(t *testing.T) {
	s, gen := state(t).StartAdmin()

	if got := s.Listing(resource.Logs).Note; got == "" {
		t.Error("the log before any answer has no note")
	}

	s, _ = s.TakeAdmin(AdminMsg{Gen: gen, Section: resource.Logs, LogLines: logLines(3, 2, 1)})
	if s.Admin.Logs.Cursor() != 3 {
		t.Fatalf("cursor = %d, want 3", s.Admin.Logs.Cursor())
	}

	s, _ = s.TakeAdmin(AdminMsg{Gen: gen, Section: resource.Logs, LogLines: logLines(5, 4), LogsSince: 3})
	if rows := s.Listing(resource.Logs).Rows; len(rows) != 5 || rows[0].Ref[0] != "5" {
		t.Fatalf("rows after following = %d, first %v", len(rows), rows[0].Ref)
	}

	// A whole-buffer answer replaces what was held.
	s, _ = s.TakeAdmin(AdminMsg{Gen: gen, Section: resource.Logs, LogLines: logLines(9)})
	if rows := s.Listing(resource.Logs).Rows; len(rows) != 1 {
		t.Errorf("rows after a refetch = %d, want 1", len(rows))
	}
}

func TestTheLogsNoteSaysHowItIsNarrowed(t *testing.T) {
	s, gen := state(t).StartAdmin()
	s, _ = s.TakeAdmin(AdminMsg{Gen: gen, Section: resource.Logs, LogLines: []client.LogLine{
		{Seq: 2, Level: "WARN", Message: "careful"}, {Seq: 1, Level: "INFO", Message: "fine"},
	}})

	s = s.WithLogs(func(v LogView) LogView {
		v.Filter = listings.LogFilter{Level: "WARN"}
		v.Follow, v.Paused = true, true
		return v
	})

	l := s.Listing(resource.Logs)
	if len(l.Rows) != 1 {
		t.Errorf("rows = %d, want only the warning", len(l.Rows))
	}
	for _, want := range []string{"1 of 2 lines", "WARN and above", "paused"} {
		if !strings.Contains(l.Note, want) {
			t.Errorf("note %q lacks %q", l.Note, want)
		}
	}
}

func TestTheAuditLogPagesOn(t *testing.T) {
	s, gen := state(t).StartAdmin()
	next := "0001"
	s, _ = s.TakeAdmin(AdminMsg{Gen: gen, Section: resource.Audit, AuditPage: &client.AuditPage{
		Records: []client.AuditRecord{{ID: "0002", OK: true}}, Next: &next,
	}})
	if s.Admin.Audit.Next != "0001" || !strings.Contains(s.Listing(resource.Audit).Note, "o loads older") {
		t.Fatalf("next = %q note %q", s.Admin.Audit.Next, s.Listing(resource.Audit).Note)
	}

	s, _ = s.TakeAdmin(AdminMsg{Gen: gen, Section: resource.Audit, AuditBefore: "0001", AuditPage: &client.AuditPage{
		Records: []client.AuditRecord{{ID: "0001", OK: true}},
	}})
	if rows := s.Listing(resource.Audit).Rows; len(rows) != 2 || s.Admin.Audit.Next != "" {
		t.Errorf("rows = %d next %q, want both pages and no more", len(rows), s.Admin.Audit.Next)
	}
}

func TestAnOlderAuditPageOvertakenByARefreshIsDropped(t *testing.T) {
	s, gen := state(t).StartAdmin()
	page := func(id, next string) *client.AuditPage {
		return &client.AuditPage{Records: []client.AuditRecord{{ID: id, OK: true}}, Next: &next}
	}

	s, _ = s.TakeAdmin(AdminMsg{Gen: gen, Section: resource.Audit, AuditPage: page("0010", "0009")})

	// An action refetches the first page, which now ends further on, before
	// the older page asked for from 0009 arrives.
	s, _ = s.TakeAdmin(AdminMsg{Gen: gen, Section: resource.Audit, AuditPage: page("0012", "0011")})
	s, _ = s.TakeAdmin(AdminMsg{Gen: gen, Section: resource.Audit, AuditBefore: "0009", AuditPage: page("0009", "0008")})

	if len(s.Admin.Audit.Records) != 1 || s.Admin.Audit.Next != "0011" {
		t.Errorf("records %+v next %q: the stale page was appended", s.Admin.Audit.Records, s.Admin.Audit.Next)
	}
}

func TestTheRouteTableReplacesTheSampleForAnAdmin(t *testing.T) {
	s := state(t).Apply(ProbedMsg{Status: up})
	if note := s.Listing(resource.API).Note; !strings.Contains(note, "kept by hand") {
		t.Errorf("a non-admin's API note = %q", note)
	}

	s, gen := s.StartAdmin()
	s, _ = s.TakeAdmin(AdminMsg{Gen: gen, Section: resource.API, Routes: "1 routes: 1 admin",
		Listing: listings.Routes([]client.Route{{Method: "GET", Path: "/_phantom/admin/v1/routes", Group: "admin"}})})

	if rows := s.Listing(resource.API).Rows; len(rows) != 1 || rows[0].Cells[1] != "/_phantom/admin/v1/routes" {
		t.Errorf("API rows = %+v", rows)
	}
	if got := cell(t, s.Listing(resource.Overview), "API routes").Cells[1]; got != "1 routes: 1 admin" {
		t.Errorf("overview API routes = %q", got)
	}
}

func TestOnlyTheIrreversibleAreConfirmedByName(t *testing.T) {
	del := Action{Kind: DeleteRoom, Target: "!r:test", Name: "#lobby:test"}
	if del.TypeToConfirm() != "#lobby:test" {
		t.Errorf("delete asks for %q", del.TypeToConfirm())
	}
	for typed, want := range map[string]bool{"#lobby:test": true, " !r:test ": true, "lobby": false, "": false} {
		if del.ConfirmedBy(typed) != want {
			t.Errorf("typing %q confirms = %v, want %v", typed, !want, want)
		}
	}

	if (Action{Kind: EraseUser, Target: "@bob:test"}).TypeToConfirm() != "@bob:test" {
		t.Error("erasing a user is not confirmed by name")
	}
	if (Action{Kind: PurgeOldMedia, Target: "local", Days: 30}).TypeToConfirm() != "local" {
		t.Error("purging local uploads does not ask for \"local\" to be typed")
	}
	if (Action{Kind: PurgeOldMedia, Target: "remote", Days: 30}).TypeToConfirm() != "" {
		t.Error("purging remote copies, which can be fetched again, asks for a name")
	}
	if (Action{Kind: BanRoom, Target: "!r:test"}).TypeToConfirm() != "" {
		t.Error("a ban, which can be lifted, asks for the name")
	}
}

func TestEveryDestructiveActionAsksFirst(t *testing.T) {
	for _, kind := range []ActionKind{
		DeactivateUser, EraseUser, SignOutDevice, RevokeToken, BanRoom, ShutdownRoom, DeleteRoom,
		DeleteMedia, PurgeRemoteMedia, DismissReport, RevokeAdmin,
	} {
		a := Action{Kind: kind, Target: "x"}
		if title, _ := a.Confirm(); title == "" && a.TypeToConfirm() == "" {
			t.Errorf("%s runs without asking", a.Label())
		}
	}
}
