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

// Routes lists every route the server registered, by audience and then
// path, as its route table answers.
func Routes(routes []client.Route) resource.Listing {
	slices.SortStableFunc(routes, func(a, b client.Route) int {
		return cmp.Or(
			strings.Compare(a.Group, b.Group),
			strings.Compare(a.Path, b.Path),
			strings.Compare(a.Method, b.Method),
		)
	})

	rows := make([]resource.Row, 0, len(routes))
	for _, r := range routes {
		stability, state := "stable", resource.Done
		if !r.Stable {
			stability, state = "unstable", resource.Held
		}

		method := r.Method
		if method == "*" {
			method = "any"
		}

		endpoint := or(r.Endpoint, "phantom's own")

		rows = append(rows, resource.Row{
			Cells: []string{method, r.Path, r.Group},
			Ref:   []string{r.Method, r.Path},
			State: state,
			Detail: []resource.Field{
				{Label: "Route", Value: method + " " + r.Path},
				{Label: "Audience", Value: r.Group},
				{Label: "Endpoint", Value: endpoint},
				{Label: "Path", Value: stability, Emphasis: state},
				{Label: "Source", Value: Source},
			},
		})
	}

	return resource.Listing{
		Sort: "audience, then path",
		Columns: []resource.Column{
			{Title: "Method", Width: 8},
			{Title: "Path", Flex: true},
			{Title: "Audience", Width: 11},
		},
		Rows: rows,
	}
}

// RouteCounts tallies a route table by audience, in words, for the
// overview: "212 routes: 160 client, 31 federation, …".
func RouteCounts(routes []client.Route) string {
	counts := map[string]int{}
	for _, r := range routes {
		counts[r.Group]++
	}

	groups := make([]string, 0, len(counts))
	for g := range counts {
		groups = append(groups, g)
	}
	slices.SortFunc(groups, func(a, b string) int {
		return cmp.Or(cmp.Compare(counts[b], counts[a]), strings.Compare(a, b))
	})

	parts := make([]string, 0, len(groups))
	for _, g := range groups {
		parts = append(parts, fmt.Sprintf("%d %s", counts[g], g))
	}

	return fmt.Sprintf("%d routes: %s", len(routes), strings.Join(parts, ", "))
}

// Audit lists the audit log as the server sent it, newest first.
func Audit(records []client.AuditRecord, now time.Time) resource.Listing {
	rows := make([]resource.Row, 0, len(records))
	for _, r := range records {
		state, outcome := resource.Done, "ok"
		if !r.OK {
			state, outcome = resource.Failed, "failed"
		}

		via := "admin API"
		if r.Via == "command" {
			via = "!admin command"
		}

		at := r.AtMs
		rows = append(rows, resource.Row{
			Cells:    []string{time.UnixMilli(at).In(now.Location()).Format("01-02 15:04"), localpart(r.Admin), r.Action, orEmpty(r.Target, "—"), outcome},
			Ref:      []string{r.ID},
			SortKeys: map[int]string{0: fmt.Sprint(at)},
			State:    state,
			Detail: []resource.Field{
				{Label: "Action", Value: r.Action},
				{Label: "Target", Value: orEmpty(r.Target, "the server")},
				{Label: "Admin", Value: r.Admin},
				{Label: "Via", Value: via},
				{Label: "When", Value: when(&at, now)},
				{Label: "Outcome", Value: outcome, Emphasis: state},
				{Label: "Result", Value: r.Result},
				{Label: "Source", Value: Source},
			},
		})
	}

	return resource.Listing{
		Sort: "newest first",
		Columns: []resource.Column{
			{Title: "Time", Width: 12},
			{Title: "Admin", Width: 12},
			{Title: "Action", Flex: true},
			{Title: "Target", Width: 26},
			{Title: "Result", Width: 7},
		},
		Rows: rows,
	}
}
