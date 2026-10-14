package inspector

import (
	"bytes"
	"encoding/json"
	"fmt"
	"strings"
	"time"

	"github.com/HilthonTT/phantom/cli/internal/client"
	"github.com/HilthonTT/phantom/cli/internal/tui/panel"
	"github.com/HilthonTT/phantom/cli/internal/tui/resource"
	"github.com/HilthonTT/phantom/cli/internal/tui/theme"
)

// The room inspector takes the workspace's place to show what the server
// holds of one room: its state, its timeline with each event's place in the
// graph, its forward extremities and the servers in it. It is what is
// reached for when a room has gone wrong, which is most often a federation
// problem, so the graph is shown as stored rather than as a client sees it.

// RoomTab is one of the inspector's views of a room.
type RoomTab int

const (
	StateTab RoomTab = iota
	TimelineTab
	ExtremitiesTab
	ServersTab

	roomTabs
)

func (t RoomTab) String() string {
	switch t {
	case StateTab:
		return "State"
	case TimelineTab:
		return "Timeline"
	case ExtremitiesTab:
		return "Extremities"
	default:
		return "Servers"
	}
}

// Item is one row of a tab, with the JSON it expands to and the fields the
// detail panel shows for it.
type Item struct {
	Cells  []string
	State  resource.State
	JSON   string
	Detail []resource.Field
}

// column lays out one of a tab's columns; a zero width takes what is left.
type column struct {
	title string
	width int
	right bool
}

var roomColumns = [roomTabs][]column{
	StateTab:       {{title: "Type", width: 26}, {title: "State key"}, {title: "Sender", width: 24}},
	TimelineTab:    {{title: "Depth", width: 7, right: true}, {title: "Type", width: 22}, {title: "Sender", width: 22}, {title: "Prev", width: 5, right: true}, {title: "Event"}},
	ExtremitiesTab: {{title: "Depth", width: 7, right: true}, {title: "Event"}, {title: "Sender", width: 22}, {title: "Sent", width: 12, right: true}},
	ServersTab:     {{title: "Server"}},
}

// Room is the inspector open on one room.
type Room struct {
	theme  theme.Theme
	glyphs theme.Glyphs

	width, height int

	RoomID  string
	title   string
	version string

	tab    RoomTab
	items  [roomTabs][]Item
	cursor [roomTabs]int

	// expanded shows the selected item's JSON beneath it.
	expanded bool

	// next asks for the timeline's older events; empty once they are all
	// held.
	next string

	loading bool
	err     string
}

func NewRoom(t theme.Theme, g theme.Glyphs) Room { return Room{theme: t, glyphs: g} }

func (r *Room) SetSize(width, height int) { r.width, r.height = width, height }

// Open starts inspecting a room, titled as its listing named it, with
// nothing held until the server answers.
func (r *Room) Open(roomID, title, version string) {
	*r = Room{theme: r.theme, glyphs: r.glyphs, width: r.width, height: r.height,
		RoomID: roomID, title: title, version: version, loading: true}
}

// Load takes everything the server sent about the room.
func (r *Room) Load(state []client.StateEvent, events client.TimelinePage, extremities []client.Extremity,
	servers []string, now time.Time) {
	r.loading, r.err = false, ""
	r.items[StateTab] = StateItems(state, now)
	r.items[TimelineTab] = TimelineItems(events.Events, now)
	r.items[ExtremitiesTab] = ExtremityItems(extremities, now)
	r.items[ServersTab] = ServerItems(servers)
	r.next = deref(events.Next)

	for _, e := range state {
		if e.Type == "m.room.create" {
			var content struct {
				Version string `json:"room_version"`
			}
			if json.Unmarshal(e.Content, &content) == nil {
				r.version = content.Version
			}
			if r.version == "" {
				// A create event without a version is version 1.
				r.version = "1"
			}
		}
	}

	for t := range r.cursor {
		r.cursor[t] = min(r.cursor[t], max(len(r.items[t])-1, 0))
	}
}

// AddOlder adds a further page of the timeline.
func (r *Room) AddOlder(events client.TimelinePage, now time.Time) {
	r.loading = false
	r.items[TimelineTab] = append(r.items[TimelineTab], TimelineItems(events.Events, now)...)
	r.next = deref(events.Next)
}

func (r *Room) Fail(err error) { r.loading, r.err = false, err.Error() }

// Loading says whether a fetch is under way, so another is not started.
func (r Room) Loading() bool { return r.loading }

// Older is where the timeline's next page starts, empty when there is none.
func (r Room) Older() string { return r.next }

func (r *Room) StartLoading() { r.loading = true }

func (r Room) Tab() RoomTab { return r.tab }

func (r *Room) NextTab() { r.tab, r.expanded = (r.tab+1)%roomTabs, false }
func (r *Room) PrevTab() { r.tab, r.expanded = (r.tab-1+roomTabs)%roomTabs, false }

func (r *Room) MoveUp()   { r.moveTo(r.cursor[r.tab] - 1) }
func (r *Room) MoveDown() { r.moveTo(r.cursor[r.tab] + 1) }
func (r *Room) PageUp()   { r.moveTo(r.cursor[r.tab] - r.listHeight()) }
func (r *Room) PageDown() { r.moveTo(r.cursor[r.tab] + r.listHeight()) }
func (r *Room) Top()      { r.moveTo(0) }
func (r *Room) Bottom()   { r.moveTo(len(r.items[r.tab]) - 1) }

func (r *Room) moveTo(i int) {
	r.cursor[r.tab] = min(max(i, 0), max(len(r.items[r.tab])-1, 0))
}

// ToggleExpand shows or hides the selected item's JSON.
func (r *Room) ToggleExpand() { r.expanded = !r.expanded }

func (r Room) Expanded() bool { return r.expanded }

// Selected is the item under the cursor, as a row for the detail panel.
func (r Room) Selected() (resource.Row, bool) {
	items := r.items[r.tab]
	if len(items) == 0 {
		return resource.Row{}, false
	}

	item := items[r.cursor[r.tab]]

	return resource.Row{Cells: item.Cells, State: item.State, Detail: item.Detail}, true
}

// Extremities is how many forward extremities the room has.
func (r Room) Extremities() int { return len(r.items[ExtremitiesTab]) }

// header lines above the list: room overview and tabs.
const headerLines = 5

func (r Room) listHeight() int { return max(r.height-2-headerLines, 1) }

func (r Room) Render(focused bool) string {
	p := panel.New(r.theme.PanelConfig(r.width, r.height, focused))
	p.SetTitle("Room · " + r.title)
	width := p.ContentWidth()

	p.AddLine(r.summary(width))
	p.AddLine(r.tabs(width))
	p.AddDivider()
	p.AddLine(r.theme.ColumnHeader.Render(r.row(r.titles(), width, " ")))

	switch {
	case r.err != "":
		p.AddLine(r.theme.StateFailed.Render("  " + r.err))
	case r.loading && len(r.items[r.tab]) == 0:
		p.AddLine(r.theme.Faint.Render("  loading…"))
	case len(r.items[r.tab]) == 0:
		p.AddLine(r.theme.Faint.Render("  nothing held"))
	default:
		p.AddLines(r.window(r.lines(width), p.Remaining())...)
	}

	info := []string{"tab switches", "enter expands", "esc closes"}
	if r.tab == TimelineTab && r.next != "" {
		info = []string{"tab switches", "enter expands", "o older", "esc closes"}
	}
	p.SetInfo(info...)

	return p.Render()
}

func (r Room) summary(width int) string {
	ext := len(r.items[ExtremitiesTab])
	extState := resource.Done
	extWord := fmt.Sprintf("%d extremities", ext)
	switch {
	case r.loading && ext == 0:
		extState, extWord = resource.NoState, "extremities …"
	case ext == 1:
		extWord = "1 extremity"
	case ext > 1:
		// More than one means the graph has forked and not merged again.
		extState = resource.Held
	}

	version := r.version
	if version == "" {
		version = "unknown"
	}

	line := r.theme.Muted.Render("  "+r.RoomID+"  ·  version "+version+"  ·  ") +
		r.theme.ForState(extState).Render(extWord) +
		r.theme.Muted.Render(fmt.Sprintf("  ·  %d servers", len(r.items[ServersTab])))

	return panel.Truncate(line, width)
}

func (r Room) tabs(width int) string {
	var b strings.Builder
	b.WriteString(r.theme.Text.Render(" "))
	for t := range roomTabs {
		label := fmt.Sprintf(" %s %d ", t, len(r.items[t]))
		if t == r.tab {
			b.WriteString(r.theme.RowSelected.Render(label))
		} else {
			b.WriteString(r.theme.Muted.Render(label))
		}
		b.WriteString(r.theme.Text.Render(" "))
	}

	return panel.Truncate(b.String(), width)
}

func (r Room) titles() []string {
	cols := roomColumns[r.tab]
	titles := make([]string, len(cols))
	for i, c := range cols {
		titles[i] = c.title
	}

	return titles
}

// row lays cells out in the tab's columns, after a two-cell gutter holding
// the cursor.
func (r Room) row(cells []string, width int, cursor string) string {
	cols := roomColumns[r.tab]

	fixed := 0
	for _, c := range cols {
		fixed += c.width + 1
	}
	flex := max(width-3-fixed, 8)

	var b strings.Builder
	b.WriteString(" " + cursor + " ")
	for i, c := range cols {
		cell := ""
		if i < len(cells) {
			cell = cells[i]
		}
		w := c.width
		if w == 0 {
			w = flex
		}
		if c.right {
			b.WriteString(panel.PadStart(cell, w))
		} else {
			b.WriteString(panel.Pad(cell, w))
		}
		b.WriteString(" ")
	}

	return panel.Truncate(b.String(), width)
}

// line is one rendered line, and whether it belongs to the selected item.
type line struct {
	text     string
	selected bool
}

func (r Room) lines(width int) []line {
	var out []line
	for i, item := range r.items[r.tab] {
		selected := i == r.cursor[r.tab]

		if selected {
			out = append(out, line{r.theme.RowSelected.Render(r.row(item.Cells, width, r.glyphs.Cursor)), true})
		} else {
			out = append(out, line{r.theme.ForState(item.State).Render(r.row(item.Cells, width, " ")), false})
		}

		if selected && r.expanded && item.JSON != "" {
			for _, l := range strings.Split(item.JSON, "\n") {
				out = append(out, line{r.theme.Muted.Render(panel.Truncate("      "+l, width)), true})
			}
		}
	}

	return out
}

// window keeps the selected item, with as much of its JSON as fits, in the
// height there is.
func (r Room) window(lines []line, height int) []string {
	first, last := -1, -1
	for i, l := range lines {
		if l.selected {
			if first < 0 {
				first = i
			}
			last = i
		}
	}

	top := 0
	if first >= 0 && last-top >= height {
		top = min(first, max(last-height+1, 0))
	}

	out := make([]string, 0, height)
	for _, l := range lines[top:min(len(lines), top+height)] {
		out = append(out, l.text)
	}

	return out
}

// StateItems are a room's state events, as the server sorted them.
func StateItems(events []client.StateEvent, now time.Time) []Item {
	items := make([]Item, 0, len(events))
	for _, e := range events {
		at := e.OriginServerTs
		items = append(items, Item{
			Cells: []string{e.Type, orDash(e.StateKey), e.Sender},
			JSON:  PrettyJSON(e.Content),
			Detail: []resource.Field{
				{Label: "Type", Value: e.Type},
				{Label: "State key", Value: orDash(e.StateKey)},
				{Label: "Sender", Value: e.Sender},
				{Label: "Event", Value: e.EventID},
				{Label: "Sent", Value: stamp(at, now)},
			},
		})
	}

	return items
}

// TimelineItems are a room's events, newest first, with each one's depth
// and the events it follows; an event following more than one is where the
// graph merged.
func TimelineItems(events []client.TimelineEvent, now time.Time) []Item {
	items := make([]Item, 0, len(events))
	for _, e := range events {
		state := resource.NoState
		if len(e.PrevEvents) > 1 {
			state = resource.Running
		}

		kind := e.Type
		if e.StateKey != nil {
			kind += " (state)"
		}

		detail := []resource.Field{
			{Label: "Event", Value: e.EventID},
			{Label: "Type", Value: e.Type},
			{Label: "Sender", Value: e.Sender},
			{Label: "Depth", Value: fmt.Sprint(e.Depth)},
			{Label: "Sent", Value: stamp(e.OriginServerTs, now)},
		}
		if e.StateKey != nil {
			detail = append(detail, resource.Field{Label: "State key", Value: orDash(*e.StateKey)})
		}
		for i, prev := range e.PrevEvents {
			label := ""
			if i == 0 {
				label = "Follows"
			}
			detail = append(detail, resource.Field{Label: label, Value: prev, Emphasis: state})
		}

		items = append(items, Item{
			Cells:  []string{fmt.Sprint(e.Depth), kind, e.Sender, fmt.Sprint(len(e.PrevEvents)), e.EventID},
			State:  state,
			JSON:   PrettyJSON(e.Content),
			Detail: detail,
		})
	}

	return items
}

// ExtremityItems are a room's forward extremities, deepest first; one the
// server lists without holding is marked as failed, since the next event
// sent would follow an event nobody here can produce.
func ExtremityItems(extremities []client.Extremity, now time.Time) []Item {
	items := make([]Item, 0, len(extremities))
	for _, e := range extremities {
		depth, sender, sent, state := "—", "not held", "—", resource.Failed
		if e.Depth != nil {
			depth, state = fmt.Sprint(*e.Depth), resource.NoState
		}
		if e.Sender != nil {
			sender = *e.Sender
		}
		if e.OriginServerTs != nil {
			sent = ago(*e.OriginServerTs, now)
		}

		items = append(items, Item{
			Cells: []string{depth, e.EventID, sender, sent},
			State: state,
			Detail: []resource.Field{
				{Label: "Event", Value: e.EventID},
				{Label: "Depth", Value: depth},
				{Label: "Sender", Value: sender, Emphasis: state},
				{Label: "Sent", Value: sent},
			},
		})
	}

	return items
}

// ServerItems are the servers with a member in the room.
func ServerItems(servers []string) []Item {
	items := make([]Item, 0, len(servers))
	for _, s := range servers {
		items = append(items, Item{
			Cells:  []string{s},
			Detail: []resource.Field{{Label: "Server", Value: s}},
		})
	}

	return items
}

// PrettyJSON indents a JSON value for reading; what does not parse is
// shown as it came.
func PrettyJSON(raw []byte) string {
	if len(raw) == 0 {
		return ""
	}

	var b bytes.Buffer
	if err := json.Indent(&b, raw, "", "  "); err != nil {
		return string(raw)
	}

	return b.String()
}

func stamp(ms int64, now time.Time) string {
	return time.UnixMilli(ms).In(now.Location()).Format("2006-01-02 15:04:05") + ", " + ago(ms, now)
}

func ago(ms int64, now time.Time) string {
	d := now.Sub(time.UnixMilli(ms))
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

func orDash(s string) string {
	if s == "" {
		return "—"
	}

	return s
}

func deref(s *string) string {
	if s == nil {
		return ""
	}

	return *s
}
