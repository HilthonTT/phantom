// Package resource is the data the interface draws: what a section is, what a
// listing's columns and rows are, and what the inspector shows for a row.
//
// Nothing here talks to a homeserver. These are the shapes the UI renders;
// [github.com/HilthonTT/phantom/cli/internal/tui/sample] fills them with
// placeholder values, and is the only place that needs replacing when the
// admin API is wired up.
package resource

// Section is one entry in the sidebar, and one listing in the workspace.
type Section int

// The sections, in sidebar order.
const (
	Overview Section = iota
	Rooms
	Users
	Federation
	Media
	Tasks
	Logs
	Settings
)

// Group is the sidebar heading a section sits under.
type Group int

// The sidebar groups, in order.
const (
	ServerGroup Group = iota
	OperationsGroup
)

// String is the section's name as the sidebar and panel titles print it.
func (s Section) String() string {
	switch s {
	case Overview:
		return "Overview"
	case Rooms:
		return "Rooms"
	case Users:
		return "Users"
	case Federation:
		return "Federation"
	case Media:
		return "Media"
	case Tasks:
		return "Tasks"
	case Logs:
		return "Logs"
	case Settings:
		return "Settings"
	default:
		return "Unknown"
	}
}

// Group is the sidebar heading the section belongs under.
func (s Section) Group() Group {
	switch s {
	case Overview, Rooms, Users, Federation, Media:
		return ServerGroup
	default:
		return OperationsGroup
	}
}

// String is the group's heading text.
func (g Group) String() string {
	switch g {
	case ServerGroup:
		return "SERVER"
	case OperationsGroup:
		return "OPERATIONS"
	default:
		return "OTHER"
	}
}

// Sections is every section, in sidebar order.
func Sections() []Section {
	return []Section{Overview, Rooms, Users, Federation, Media, Tasks, Logs, Settings}
}

// Column is one field of a listing.
type Column struct {
	// Title is the column header, printed in caps.
	Title string

	// Width is how many columns it occupies. Ignored when Flex is set.
	Width int

	// Flex marks the one column that absorbs whatever width is left over. A
	// listing should have exactly one; without it the row is left-packed and
	// the remaining space goes unused.
	Flex bool

	// Right aligns the cell against its right edge, which is what a count or a
	// size wants.
	Right bool
}

// Row is one entry in a listing.
type Row struct {
	// Cells are positional: one per column of the listing, in order.
	Cells []string

	// Detail is what the inspector shows while this row is under the cursor.
	Detail []Field

	// State tints the row and the marker beside it. Use [NoState] where the
	// row is not a thing that succeeds or fails.
	State State

	// Marked is set on rows the user has picked out for a bulk operation.
	Marked bool
}

// Field is one labelled value in the inspector or the summary box.
type Field struct {
	Label string
	Value string

	// Emphasis tints the value. Use [NoState] for an ordinary one.
	Emphasis State
}

// Listing is one section's table.
type Listing struct {
	Columns []Column
	Rows    []Row

	// Sort is how the rows are ordered, printed in the panel's bottom border.
	Sort string
}

// State is how something is doing, and picks the colour it is drawn in.
type State int

// The states. NoState draws in the ordinary text colour.
const (
	NoState State = iota
	Running
	Done
	Failed
	Held
)

// Task is one long-running admin operation, as the task bar draws it.
type Task struct {
	Name  string
	State State

	// Progress is in [0,1]. It is ignored for a task that is not [Running].
	Progress float64

	// Note is the second line under the name — what the task is working on.
	Note string
}

// Server is what the connection box reports about the homeserver the CLI is
// pointed at.
type Server struct {
	Name    string
	URL     string
	Version string
	Admin   string
	State   State

	// Status is the word printed beside the state dot.
	Status string

	// Facts are shown under the address when the box has the room.
	Facts []Field
}
