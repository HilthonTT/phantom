package listings

import (
	"testing"

	"github.com/HilthonTT/phantom/cli/internal/client"
)

func line(seq int64, level, target, message string) client.LogLine {
	return client.LogLine{Seq: seq, Level: level, Target: target, Message: message}
}

func TestALogFilterKeepsWhatItNames(t *testing.T) {
	sending := line(1, "WARN", "phantom_service::net::sending", "Retrying matrix.org")
	debug := line(2, "DEBUG", "phantom_api::router", "GET /sync")
	errLine := line(3, "ERROR", "phantom_service::media", "disk full")

	for _, tc := range []struct {
		name   string
		filter LogFilter
		want   []bool
	}{
		{"nothing", LogFilter{}, []bool{true, true, true}},
		{"warn and above", LogFilter{Level: "WARN"}, []bool{true, false, true}},
		{"a target, in any case", LogFilter{Target: "SENDING"}, []bool{true, false, false}},
		{"text in the message", LogFilter{Text: "matrix.org"}, []bool{true, false, false}},
		{"text in the target", LogFilter{Text: "media"}, []bool{false, false, true}},
		{"all of them at once", LogFilter{Level: "INFO", Target: "router"}, []bool{false, false, false}},
	} {
		for i, l := range []client.LogLine{sending, debug, errLine} {
			if got := tc.filter.Match(l); got != tc.want[i] {
				t.Errorf("%s: line %d kept = %v, want %v", tc.name, i, got, tc.want[i])
			}
		}
	}
}

func TestTheLevelStepsRoundToEveryLine(t *testing.T) {
	var f LogFilter
	var seen []string
	for range 5 {
		f = f.NextLevel()
		seen = append(seen, f.Level)
	}

	want := []string{"INFO", "WARN", "ERROR", "", "INFO"}
	for i := range want {
		if seen[i] != want[i] {
			t.Fatalf("levels = %q, want %q", seen, want)
		}
	}
}

func TestAFilterDescribesItself(t *testing.T) {
	if d := (LogFilter{}).Describe(); d != "" {
		t.Errorf("an empty filter reads %q", d)
	}
	if d := (LogFilter{Level: "WARN", Target: "sending"}).Describe(); d != `WARN and above · target "sending"` {
		t.Errorf("describe = %q", d)
	}
}

func TestMergingTheLogKeepsEachLineOnceNewestFirst(t *testing.T) {
	held := []client.LogLine{line(5, "INFO", "", "e"), line(4, "INFO", "", "d")}
	fresh := []client.LogLine{line(7, "INFO", "", "g"), line(6, "INFO", "", "f"), line(5, "INFO", "", "e")}

	got := MergeLogs(held, fresh, 4)
	want := []int64{7, 6, 5, 4}
	if len(got) != len(want) {
		t.Fatalf("merged %d lines, want %d", len(got), len(want))
	}
	for i := range want {
		if got[i].Seq != want[i] {
			t.Errorf("line %d = %d, want %d", i, got[i].Seq, want[i])
		}
	}

	if got := MergeLogs(held, nil, 1); len(got) != 1 || got[0].Seq != 5 {
		t.Errorf("capped merge = %+v", got)
	}
}

func TestAServerThatStartedNumberingOverReplacesTheLog(t *testing.T) {
	held := []client.LogLine{line(800, "INFO", "", "old"), line(799, "INFO", "", "older")}
	fresh := []client.LogLine{line(2, "INFO", "", "b"), line(1, "INFO", "", "a")}

	got := MergeLogs(held, fresh, 10)
	if len(got) != 2 || got[0].Seq != 2 || got[1].Seq != 1 {
		t.Errorf("merge after a restart = %+v, want the fresh lines alone", got)
	}
}

func TestFilteringTheLogKeepsItsOrder(t *testing.T) {
	lines := []client.LogLine{line(3, "ERROR", "", "c"), line(2, "INFO", "", "b"), line(1, "WARN", "", "a")}

	got := FilterLogs(lines, LogFilter{Level: "WARN"})
	if len(got) != 2 || got[0].Seq != 3 || got[1].Seq != 1 {
		t.Errorf("filtered = %+v", got)
	}
}
