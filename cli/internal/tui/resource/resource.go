package resource

type Section int

const (
	Overview Section = iota
	Services
	Rooms
	Users
	Tokens
	Federation
	Media
	Tasks
	Logs
	Settings
)

type Group int

const (
	ServerGroup Group = iota
	OperationsGroup
)

func (s Section) String() string {
	switch s {
	case Overview:
		return "Overview"
	case Services:
		return "Services"
	case Rooms:
		return "Rooms"
	case Users:
		return "Users"
	case Tokens:
		return "Tokens"
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

func (s Section) Group() Group {
	switch s {
	case Overview, Services, Rooms, Users, Tokens, Federation, Media:
		return ServerGroup
	default:
		return OperationsGroup
	}
}

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

func Sections() []Section {
	return []Section{
		Overview, Services, Rooms, Users, Tokens, Federation, Media,
		Tasks, Logs, Settings,
	}
}

type Column struct {
	Title string

	Width int

	Flex bool

	Right bool
}

type Row struct {
	Cells []string

	Detail []Field

	State State

	Marked bool
}

type Field struct {
	Label string
	Value string

	Emphasis State
}

type Listing struct {
	Columns []Column
	Rows    []Row

	Sort string
}

type State int

const (
	NoState State = iota
	Running
	Done
	Failed
	Held
)

type Task struct {
	Name  string
	State State

	Progress float64

	Note string
}

type Server struct {
	Name    string
	URL     string
	Version string
	Admin   string
	State   State

	Status string

	Facts []Field
}
