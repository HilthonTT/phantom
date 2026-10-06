// Package rooms folds /sync responses into the rooms the signed-in user is
// in, and draws them as the chat panel's channels.
package rooms

import (
	"cmp"
	"encoding/json"
	"slices"
	"strings"

	"github.com/HilthonTT/phantom/cli/internal/client"
	"github.com/HilthonTT/phantom/cli/internal/tui/resource"
)

// timelineCap bounds the events kept per room; the oldest are dropped.
const timelineCap = 500

// Store is the user's joined rooms as the syncs so far have described them.
type Store struct {
	self string

	rooms map[string]*room

	// direct is every room some m.direct entry names.
	direct map[string]bool

	presence map[string]resource.Presence

	// pending are messages sent from here, by room, not yet echoed by a sync.
	pending map[string][]Pending
}

// Pending is a message sent from here that a sync has not echoed yet.
type Pending struct {
	TxnID   string
	Message resource.Message
}

type room struct {
	id string

	// state is the room's current state: event type, then state key.
	state map[string]map[string]client.Event

	timeline []client.Event

	heroes []string
	typing []string

	// receipts is each user's m.read receipt, by user.
	receipts map[string]string

	unread int
}

func New(self string) *Store {
	return &Store{
		self:     self,
		rooms:    map[string]*room{},
		direct:   map[string]bool{},
		presence: map[string]resource.Presence{},
		pending:  map[string][]Pending{},
	}
}

func (s *Store) Self() string { return s.self }

// Apply folds one sync response in.
func (s *Store) Apply(resp client.SyncResponse) {
	for _, ev := range resp.AccountData.Events {
		if ev.Type == "m.direct" {
			s.applyDirect(ev)
		}
	}

	for _, ev := range resp.Presence.Events {
		s.presence[ev.Sender] = presenceOf(ev)
	}

	for id := range resp.Rooms.Leave {
		delete(s.rooms, id)
		delete(s.pending, id)
	}

	for id, joined := range resp.Rooms.Join {
		r, ok := s.rooms[id]
		if !ok {
			r = &room{id: id, state: map[string]map[string]client.Event{}, receipts: map[string]string{}}
			s.rooms[id] = r
		}

		s.applyRoom(r, joined)
	}
}

func (s *Store) applyDirect(ev client.Event) {
	var content map[string][]string
	if json.Unmarshal(ev.Content, &content) != nil {
		return
	}

	s.direct = map[string]bool{}
	for _, ids := range content {
		for _, id := range ids {
			s.direct[id] = true
		}
	}
}

func (s *Store) applyRoom(r *room, joined client.JoinedRoom) {
	if len(joined.Summary.Heroes) > 0 {
		r.heroes = joined.Summary.Heroes
	}

	for _, ev := range joined.State.Events {
		r.setState(ev)
	}

	seen := make(map[string]bool, len(r.timeline))
	for _, ev := range r.timeline {
		seen[ev.EventID] = true
	}

	for _, ev := range joined.Timeline.Events {
		if ev.StateKey != nil {
			r.setState(ev)
		}
		if ev.Unsigned.TransactionID != "" {
			s.settle(r.id, ev.Unsigned.TransactionID)
		}
		if !seen[ev.EventID] {
			seen[ev.EventID] = true
			r.timeline = append(r.timeline, ev)
		}
	}

	if over := len(r.timeline) - timelineCap; over > 0 {
		r.timeline = slices.Clone(r.timeline[over:])
	}

	for _, ev := range joined.Ephemeral.Events {
		switch ev.Type {
		case "m.typing":
			var content struct {
				UserIDs []string `json:"user_ids"`
			}
			if json.Unmarshal(ev.Content, &content) == nil {
				r.typing = content.UserIDs
			}

		case "m.receipt":
			var content map[string]map[string]map[string]json.RawMessage
			if json.Unmarshal(ev.Content, &content) != nil {
				continue
			}
			for eventID, kinds := range content {
				for user := range kinds["m.read"] {
					r.receipts[user] = eventID
				}
			}
		}
	}

	r.unread = joined.UnreadNotifications.NotificationCount
}

func (r *room) setState(ev client.Event) {
	if ev.StateKey == nil {
		return
	}

	byKey, ok := r.state[ev.Type]
	if !ok {
		byKey = map[string]client.Event{}
		r.state[ev.Type] = byKey
	}
	byKey[*ev.StateKey] = ev
}

// Send records a message sent from here until a sync echoes it.
func (s *Store) Send(roomID, txnID string, msg resource.Message) {
	msg.Pending = true
	s.pending[roomID] = append(s.pending[roomID], Pending{TxnID: txnID, Message: msg})
}

// Fail marks a message the server refused.
func (s *Store) Fail(roomID, txnID string) {
	for i, p := range s.pending[roomID] {
		if p.TxnID == txnID {
			s.pending[roomID][i].Message.Pending = false
			s.pending[roomID][i].Message.Failed = true
		}
	}
}

func (s *Store) settle(roomID, txnID string) {
	s.pending[roomID] = slices.DeleteFunc(s.pending[roomID], func(p Pending) bool {
		return p.TxnID == txnID
	})
}

// MarkRead clears a room's unread count until the server's next count.
func (s *Store) MarkRead(roomID string) {
	if r, ok := s.rooms[roomID]; ok {
		r.unread = 0
	}
}

// Latest is the newest event of a room, which a read receipt is put on.
func (s *Store) Latest(roomID string) string {
	r, ok := s.rooms[roomID]
	if !ok || len(r.timeline) == 0 {
		return ""
	}

	return r.timeline[len(r.timeline)-1].EventID
}

// Channels draws every room: rooms first, then direct messages, each newest
// activity first.
func (s *Store) Channels() []resource.Channel {
	type ranked struct {
		ch   resource.Channel
		last int64
	}

	all := make([]ranked, 0, len(s.rooms))
	for _, r := range s.rooms {
		var last int64
		if n := len(r.timeline); n > 0 {
			last = r.timeline[n-1].OriginServerTS
		}
		all = append(all, ranked{ch: s.channel(r), last: last})
	}

	slices.SortFunc(all, func(a, b ranked) int {
		if a.ch.Direct != b.ch.Direct {
			if a.ch.Direct {
				return 1
			}
			return -1
		}
		if c := cmp.Compare(b.last, a.last); c != 0 {
			return c
		}
		return strings.Compare(a.ch.Name, b.ch.Name)
	})

	out := make([]resource.Channel, len(all))
	for i, r := range all {
		out[i] = r.ch
	}

	return out
}

func (s *Store) channel(r *room) resource.Channel {
	members := s.members(r)

	ch := resource.Channel{
		ID:        r.id,
		Topic:     r.stringState("m.room.topic", "topic"),
		Direct:    s.direct[r.id],
		Encrypted: r.has("m.room.encryption"),
		Unread:    r.unread,
		Typing:    r.typing,
		Members:   members,
		Messages:  s.messages(r, members),
	}
	ch.Name = s.name(r, members, ch.Direct)

	if latest := s.Latest(r.id); latest != "" {
		for user, eventID := range r.receipts {
			if eventID == latest && user != s.self {
				ch.ReadBy = append(ch.ReadBy, user)
			}
		}
		slices.Sort(ch.ReadBy)
	}

	for _, p := range s.pending[r.id] {
		ch.Messages = append(ch.Messages, p.Message)
	}

	return ch
}

// name follows the spec's room naming: the m.room.name, else the canonical
// alias, else the other members, else the room ID.
func (s *Store) name(r *room, members []resource.Member, direct bool) string {
	if name := r.stringState("m.room.name", "name"); name != "" {
		return name
	}
	if alias := r.stringState("m.room.canonical_alias", "alias"); alias != "" && !direct {
		return alias
	}

	var others []string
	for _, id := range r.heroes {
		if id != s.self {
			others = append(others, displayName(members, id))
		}
	}
	if len(others) == 0 {
		for _, m := range members {
			if m.ID != s.self {
				others = append(others, m.Name)
			}
		}
	}

	switch len(others) {
	case 0:
		return r.id
	case 1, 2, 3:
		return strings.Join(others, ", ")
	default:
		return strings.Join(others[:3], ", ") + " and others"
	}
}

func (s *Store) members(r *room) []resource.Member {
	levels := r.powerLevels()

	var members []resource.Member
	for id, ev := range r.state["m.room.member"] {
		var content memberContent
		if json.Unmarshal(ev.Content, &content) != nil || content.Membership != "join" {
			continue
		}

		name := content.DisplayName
		if name == "" {
			name = localpart(id)
		}

		presence, ok := s.presence[id]
		if !ok && id == s.self {
			presence = resource.Online
		}

		members = append(members, resource.Member{
			ID: id, Name: name, Presence: presence, Admin: levels.of(id) >= adminLevel,
		})
	}

	slices.SortFunc(members, func(a, b resource.Member) int { return strings.Compare(a.Name, b.Name) })

	return members
}

const adminLevel = 100

type powerLevels struct {
	Users        map[string]int `json:"users"`
	UsersDefault int            `json:"users_default"`
}

func (p powerLevels) of(user string) int {
	if level, ok := p.Users[user]; ok {
		return level
	}

	return p.UsersDefault
}

func (r *room) powerLevels() powerLevels {
	var levels powerLevels
	if ev, ok := r.state["m.room.power_levels"][""]; ok {
		_ = json.Unmarshal(ev.Content, &levels)
	}

	return levels
}

func (r *room) has(eventType string) bool {
	_, ok := r.state[eventType][""]
	return ok
}

func (r *room) stringState(eventType, field string) string {
	ev, ok := r.state[eventType][""]
	if !ok {
		return ""
	}

	var content map[string]any
	if json.Unmarshal(ev.Content, &content) != nil {
		return ""
	}

	value, _ := content[field].(string)

	return value
}

func presenceOf(ev client.Event) resource.Presence {
	var content struct {
		Presence string `json:"presence"`
	}
	_ = json.Unmarshal(ev.Content, &content)

	switch content.Presence {
	case "online":
		return resource.Online
	case "unavailable":
		return resource.Idle
	default:
		return resource.Offline
	}
}

func displayName(members []resource.Member, id string) string {
	for _, m := range members {
		if m.ID == id {
			return m.Name
		}
	}

	return localpart(id)
}

func localpart(id string) string {
	id = strings.TrimPrefix(id, "@")
	if name, _, ok := strings.Cut(id, ":"); ok {
		return name
	}

	return id
}

// PendingTxns are the transaction IDs of a room's messages awaiting their
// echo, oldest first.
func (s *Store) PendingTxns(roomID string) []string {
	ids := make([]string, 0, len(s.pending[roomID]))
	for _, p := range s.pending[roomID] {
		ids = append(ids, p.TxnID)
	}

	return ids
}
