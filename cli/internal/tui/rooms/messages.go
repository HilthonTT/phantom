package rooms

import (
	"encoding/json"
	"slices"
	"time"

	"github.com/HilthonTT/phantom/cli/internal/client"
	"github.com/HilthonTT/phantom/cli/internal/tui/resource"
)

type relatesTo struct {
	RelType string `json:"rel_type"`
	EventID string `json:"event_id"`
	Key     string `json:"key"`
}

type messageContent struct {
	MsgType   string     `json:"msgtype"`
	Body      string     `json:"body"`
	RelatesTo *relatesTo `json:"m.relates_to"`

	NewContent *struct {
		Body string `json:"body"`
	} `json:"m.new_content"`
}

type memberContent struct {
	Membership  string `json:"membership"`
	DisplayName string `json:"displayname"`
	Reason      string `json:"reason"`
}

// relations is what the timeline says about its own events: which were
// redacted, edited, reacted to, or replied to in a thread.
type relations struct {
	redacted map[string]bool

	// edits is each edited event's newest body.
	edits map[string]string

	// reactions is each event's reaction keys in the order first seen, with
	// who reacted.
	reactions map[string][]reaction

	replies map[string]int
}

type reaction struct {
	key     string
	senders []string
}

func (s *Store) relations(r *room) relations {
	rel := relations{
		redacted:  map[string]bool{},
		edits:     map[string]string{},
		reactions: map[string][]reaction{},
		replies:   map[string]int{},
	}

	for _, ev := range r.timeline {
		if ev.Type != "m.room.redaction" {
			continue
		}

		target := ev.Redacts
		if target == "" {
			var content struct {
				Redacts string `json:"redacts"`
			}
			_ = json.Unmarshal(ev.Content, &content)
			target = content.Redacts
		}
		rel.redacted[target] = true
	}

	for _, ev := range r.timeline {
		if rel.redacted[ev.EventID] || len(ev.Unsigned.RedactedBecause) > 0 {
			rel.redacted[ev.EventID] = true
			continue
		}

		var content messageContent
		if json.Unmarshal(ev.Content, &content) != nil || content.RelatesTo == nil {
			continue
		}
		to := content.RelatesTo

		switch {
		case ev.Type == "m.reaction" && to.RelType == "m.annotation":
			rel.react(to.EventID, to.Key, ev.Sender)

		case ev.Type == "m.room.message" && to.RelType == "m.replace" && content.NewContent != nil:
			rel.edits[to.EventID] = content.NewContent.Body

		case to.RelType == "m.thread":
			rel.replies[to.EventID]++
		}
	}

	return rel
}

func (rel relations) react(target, key, sender string) {
	list := rel.reactions[target]
	for i := range list {
		if list[i].key == key {
			if !slices.Contains(list[i].senders, sender) {
				list[i].senders = append(list[i].senders, sender)
			}
			return
		}
	}

	rel.reactions[target] = append(list, reaction{key: key, senders: []string{sender}})
}

// messages draws a room's timeline as the chat shows it: thread replies,
// edits and reactions fold into the events they relate to.
func (s *Store) messages(r *room, members []resource.Member) []resource.Message {
	rel := s.relations(r)

	var out []resource.Message
	for _, ev := range r.timeline {
		msg, ok := s.message(ev, rel, members)
		if !ok {
			continue
		}

		msg.EventID = ev.EventID
		msg.Time = time.UnixMilli(ev.OriginServerTS).Local().Format("15:04")
		msg.Replies = rel.replies[ev.EventID]

		for _, re := range rel.reactions[ev.EventID] {
			msg.Reactions = append(msg.Reactions, resource.Reaction{
				Key: re.key, Count: len(re.senders), Mine: slices.Contains(re.senders, s.self),
			})
		}

		out = append(out, msg)
	}

	return out
}

func (s *Store) message(ev client.Event, rel relations, members []resource.Member) (resource.Message, bool) {
	msg := resource.Message{Sender: ev.Sender}

	switch ev.Type {
	case "m.room.message":
		if rel.redacted[ev.EventID] {
			msg.Redacted = true
			return msg, true
		}

		var content messageContent
		if json.Unmarshal(ev.Content, &content) != nil {
			return msg, false
		}
		if to := content.RelatesTo; to != nil && (to.RelType == "m.replace" || to.RelType == "m.thread") {
			return msg, false
		}

		msg.Body = content.Body
		if edit, ok := rel.edits[ev.EventID]; ok {
			msg.Body, msg.Edited = edit, true
		}

		switch content.MsgType {
		case "m.emote":
			msg.Kind = resource.Emote
		case "m.notice":
			msg.Kind = resource.Notice
		}

		return msg, true

	case "m.room.encrypted":
		if rel.redacted[ev.EventID] {
			msg.Redacted = true
			return msg, true
		}
		msg.Kind = resource.Notice
		msg.Body = "encrypted message; this console cannot decrypt it"
		return msg, true

	case "m.room.member":
		return s.membership(ev, members)

	case "m.room.name":
		return change(ev, "name", "renamed the room to ")

	case "m.room.topic":
		return change(ev, "topic", "changed the topic to ")
	}

	return msg, false
}

// membership describes a membership change; the message names its subject,
// which the chat puts in front of what happened to them.
func (s *Store) membership(ev client.Event, members []resource.Member) (resource.Message, bool) {
	if ev.StateKey == nil {
		return resource.Message{}, false
	}

	var now, before memberContent
	if json.Unmarshal(ev.Content, &now) != nil {
		return resource.Message{}, false
	}
	_ = json.Unmarshal(ev.Unsigned.PrevContent, &before)

	subject := *ev.StateKey
	by := displayName(members, ev.Sender)
	msg := resource.Message{Sender: subject, Kind: resource.Membership}

	switch now.Membership {
	case "join":
		switch {
		case before.Membership != "join":
			msg.Body = "joined the room"
		case now.DisplayName != before.DisplayName && now.DisplayName != "":
			msg.Body = "is now known as " + now.DisplayName
		default:
			return msg, false
		}
	case "invite":
		msg.Body = "was invited by " + by
	case "knock":
		msg.Body = "asked to join"
	case "ban":
		msg.Body = "was banned by " + by
	case "leave":
		switch {
		case ev.Sender != subject && before.Membership == "ban":
			msg.Body = "was unbanned by " + by
		case ev.Sender != subject && before.Membership == "invite":
			msg.Body = "had their invite withdrawn by " + by
		case ev.Sender != subject:
			msg.Body = "was removed by " + by
		case before.Membership == "invite":
			msg.Body = "declined the invite"
		default:
			msg.Body = "left the room"
		}
	default:
		return msg, false
	}

	if now.Reason != "" && now.Membership != "join" {
		msg.Body += ": " + now.Reason
	}

	return msg, true
}

func change(ev client.Event, field, says string) (resource.Message, bool) {
	var content map[string]any
	if json.Unmarshal(ev.Content, &content) != nil {
		return resource.Message{}, false
	}

	value, _ := content[field].(string)
	if value == "" {
		return resource.Message{}, false
	}

	return resource.Message{Sender: ev.Sender, Kind: resource.Membership, Body: says + value}, true
}
