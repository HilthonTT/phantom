package resource

type Presence int

const (
	Offline Presence = iota
	Idle
	Online
)

type Member struct {
	ID   string
	Name string

	Presence Presence

	Admin bool
}

type MessageKind int

const (
	Text MessageKind = iota
	Emote
	Notice
	Membership
)

type Message struct {
	Time   string
	Sender string
	Body   string

	Kind MessageKind
}

type Channel struct {
	Name  string
	Topic string

	Direct    bool
	Encrypted bool

	Unread int

	Members  []Member
	Messages []Message

	Draft string
}
