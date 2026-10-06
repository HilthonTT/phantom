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
	// EventID is empty for a sample message and one still being sent.
	EventID string

	Time   string
	Sender string
	Body   string

	Kind MessageKind

	Edited   bool
	Redacted bool

	// Pending is a message sent from here the server has not echoed yet, and
	// Failed one it refused.
	Pending bool
	Failed  bool

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
	// ID is the Matrix room ID, empty for a sample room.
	ID string

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
}
