package listings

import (
	"fmt"
	"strings"

	"github.com/HilthonTT/phantom/cli/internal/client"
)

// LogFilter narrows the log to what is being looked for. Its zero value
// keeps every line.
type LogFilter struct {
	// Level is the least severe level kept: "" for every line, else DEBUG,
	// INFO, WARN or ERROR.
	Level string

	// Target and Text are substrings, matched without regard to case: Target
	// against the line's module, Text against its message, module and span.
	Target string
	Text   string
}

// logLevels are the levels a filter steps through, least severe first; ""
// keeps everything.
var logLevels = []string{"", "INFO", "WARN", "ERROR"}

func severity(level string) int {
	switch strings.ToUpper(level) {
	case "ERROR":
		return 4
	case "WARN":
		return 3
	case "INFO":
		return 2
	case "DEBUG":
		return 1
	default:
		return 0
	}
}

// Match says whether the filter keeps a line.
func (f LogFilter) Match(l client.LogLine) bool {
	if f.Level != "" && severity(l.Level) < severity(f.Level) {
		return false
	}
	if f.Target != "" && !containsFold(l.Target, f.Target) {
		return false
	}
	if f.Text != "" && !containsFold(l.Message, f.Text) &&
		!containsFold(l.Target, f.Text) && !containsFold(l.Span, f.Text) {
		return false
	}

	return true
}

// NextLevel steps the filter to the next stricter level, back round to
// every line after ERROR.
func (f LogFilter) NextLevel() LogFilter {
	for i, level := range logLevels {
		if level == f.Level {
			f.Level = logLevels[(i+1)%len(logLevels)]
			return f
		}
	}

	f.Level = ""
	return f
}

// Active says whether the filter leaves out anything.
func (f LogFilter) Active() bool { return f != LogFilter{} }

// Describe is the filter in words, for the listing's footer: empty when it
// keeps every line.
func (f LogFilter) Describe() string {
	var parts []string
	if f.Level != "" {
		parts = append(parts, f.Level+" and above")
	}
	if f.Target != "" {
		parts = append(parts, fmt.Sprintf("target %q", f.Target))
	}
	if f.Text != "" {
		parts = append(parts, fmt.Sprintf("matching %q", f.Text))
	}

	return strings.Join(parts, " · ")
}

// FilterLogs is the lines the filter keeps, in their order.
func FilterLogs(lines []client.LogLine, f LogFilter) []client.LogLine {
	if !f.Active() {
		return lines
	}

	kept := make([]client.LogLine, 0, len(lines))
	for _, l := range lines {
		if f.Match(l) {
			kept = append(kept, l)
		}
	}

	return kept
}

// MergeLogs lays lines newly fetched, newest first, over the ones already
// held, also newest first: a line already held is not repeated, and only
// the newest limit are kept. Fresh lines numbered below everything held mean
// the server's numbering started over, so they replace what is held rather
// than being dropped as already seen.
func MergeLogs(held, fresh []client.LogLine, limit int) []client.LogLine {
	newest := int64(-1)
	if len(held) > 0 {
		newest = held[0].Seq
	}

	if len(fresh) > 0 && fresh[0].Seq < newest {
		held, newest = nil, -1
	}

	merged := make([]client.LogLine, 0, min(len(fresh)+len(held), limit))
	for _, l := range fresh {
		if l.Seq > newest {
			merged = append(merged, l)
		}
	}
	merged = append(merged, held...)

	return merged[:min(len(merged), limit)]
}

func containsFold(s, sub string) bool {
	return strings.Contains(strings.ToLower(s), strings.ToLower(sub))
}
