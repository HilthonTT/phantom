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

	Edited   bool
	Redacted bool

	Reactions []Reaction

	// Replies is how many messages the thread rooted at this one holds.
	Replies int
}

type Reaction struct {
	Key   string
	Count int

	// Mine is whether the user reading the room sent one of them.
	Mine bool
}

type Channel struct {
	Name  string
	Topic string

	Direct    bool
	Encrypted bool

	Unread int

	// Typing and ReadBy are the IDs of the members typing now, and of those
	// whose read receipt sits on the newest message.
	Typing []string
	ReadBy []string

	Members  []Member
	Messages []Message

	Draft string
}
