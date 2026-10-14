package inspector

import (
	"strings"
	"testing"
	"time"

	"github.com/HilthonTT/phantom/cli/internal/client"
	"github.com/HilthonTT/phantom/cli/internal/tui/resource"
	"github.com/HilthonTT/phantom/cli/internal/tui/theme"
)

var now = time.Date(2026, 10, 9, 12, 0, 0, 0, time.UTC)

func room(t *testing.T) Room {
	t.Helper()

	r := NewRoom(theme.Default(), theme.UnicodeGlyphs())
	r.SetSize(90, 20)
	r.Open("!r:test", "#lobby:test", "")

	depth := int64(41)
	sender := "@bob:test"
	r.Load(
		[]client.StateEvent{
			{EventID: "$create", Type: "m.room.create", Sender: "@alice:test", Content: client.RawJSON(`{"room_version":"12"}`)},
			{EventID: "$name", Type: "m.room.name", Sender: "@alice:test", Content: client.RawJSON(`{"name":"Lobby"}`)},
		},
		client.TimelinePage{Events: []client.TimelineEvent{
			{EventID: "$merge", Type: "m.room.message", Sender: "@bob:test", Depth: 42, PrevEvents: []string{"$a", "$b"}, Content: client.RawJSON(`{"body":"hi"}`)},
			{EventID: "$a", Type: "m.room.message", Sender: "@bob:test", Depth: 41, PrevEvents: []string{"$name"}},
		}},
		[]client.Extremity{{EventID: "$merge", Depth: &depth, Sender: &sender}, {EventID: "$lost"}},
		[]string{"remote.example", "test"},
		now,
	)

	return r
}

func TestTheInspectorReadsTheRoomVersionFromItsCreateEvent(t *testing.T) {
	r := room(t)

	if r.version != "12" || r.Extremities() != 2 {
		t.Errorf("version %q, %d extremities", r.version, r.Extremities())
	}
}

func TestAMergeAndAnUnheldExtremityStandOut(t *testing.T) {
	r := room(t)

	if items := r.items[TimelineTab]; items[0].State != resource.Running || items[1].State != resource.NoState {
		t.Errorf("timeline states = %d, %d; want the merge marked", items[0].State, items[1].State)
	}
	if items := r.items[ExtremitiesTab]; items[1].State != resource.Failed || items[1].Cells[2] != "not held" {
		t.Errorf("an extremity the server does not hold reads %+v", items[1])
	}
}

func TestExpandingShowsTheEventsJSON(t *testing.T) {
	r := room(t)
	r.NextTab() // the timeline

	if strings.Contains(r.Render(true), `"body": "hi"`) {
		t.Fatal("the JSON shows before it is expanded")
	}

	r.ToggleExpand()
	if !strings.Contains(r.Render(true), `"body": "hi"`) {
		t.Error("the expanded event's JSON is not shown")
	}

	r.NextTab()
	if r.Expanded() {
		t.Error("switching views keeps the expansion")
	}
}

func TestTheSelectedEventFeedsTheDetailPanel(t *testing.T) {
	r := room(t)
	r.NextTab()
	r.MoveDown()

	row, ok := r.Selected()
	if !ok || row.Detail[0].Value != "$a" {
		t.Errorf("selected = %+v", row)
	}

	r.MoveDown() // past the end stays on the last
	if row, _ := r.Selected(); row.Detail[0].Value != "$a" {
		t.Errorf("the cursor left the list: %+v", row)
	}
}

func TestOlderEventsAreAddedAfterTheNewer(t *testing.T) {
	r := room(t)
	next := "s123"
	r.AddOlder(client.TimelinePage{Events: []client.TimelineEvent{{EventID: "$old", Depth: 1}}, Next: &next}, now)

	if items := r.items[TimelineTab]; len(items) != 3 || items[2].Detail[0].Value != "$old" || r.Older() != "s123" {
		t.Errorf("timeline = %d items, older %q", len(items), r.Older())
	}
}

func TestPrettyJSONIndentsOrKeepsWhatItCannotRead(t *testing.T) {
	if got := PrettyJSON([]byte(`{"a":1}`)); got != "{\n  \"a\": 1\n}" {
		t.Errorf("pretty = %q", got)
	}
	if got := PrettyJSON([]byte(`not json`)); got != "not json" {
		t.Errorf("unreadable = %q", got)
	}
}
