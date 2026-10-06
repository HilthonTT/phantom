package chat

import (
	"strings"
	"time"

	"charm.land/bubbles/v2/textinput"
	tea "charm.land/bubbletea/v2"

	"github.com/HilthonTT/phantom/cli/internal/tui/resource"
	"github.com/HilthonTT/phantom/cli/internal/tui/sample"
	"github.com/HilthonTT/phantom/cli/internal/tui/theme"
)

const ChannelsWidth = 26

const MinConversationWidth = 40

const emotePrefix = "/me "

type Model struct {
	theme  theme.Theme
	glyphs theme.Glyphs

	self     string
	channels []resource.Channel
	cursor   int

	// drafts is what was being written in each room left mid-sentence, by
	// room key.
	drafts map[string]string

	// live is set once the rooms come from the server rather than the sample.
	live bool

	// status is a note on the channel list, such as a failing sync.
	status string

	scroll int

	composer  textinput.Model
	composing bool

	width  int
	height int
}

func New(t theme.Theme, g theme.Glyphs) Model {
	m := Model{
		theme:    t,
		glyphs:   g,
		self:     sample.Self,
		channels: sample.Channels(),
		drafts:   map[string]string{},
		composer: t.Input(" "+g.Arrow+" ", "", t.Palette.Surface),
	}
	m.composer.Placeholder = m.placeholder()

	return m
}

func (m *Model) SetSize(width, height int) {
	m.width, m.height = width, height
	m.composer.SetWidth(max(m.conversationWidth()-8, 4))
	m.clampScroll()
}

func (m Model) Channels() []resource.Channel { return m.channels }

// noRooms stands in for the open room when the user is in none.
var noRooms = resource.Channel{
	Name:  "no rooms",
	Topic: "You are in no rooms yet. Join one with :join #room:server.",
}

func (m Model) Channel() resource.Channel {
	if len(m.channels) == 0 {
		return noRooms
	}

	return m.channels[m.cursor]
}

func (m Model) Live() bool { return m.live }

func (m Model) Self() string { return m.self }

// SetLive shows the server's rooms for self, keeping the open room open when
// it is still there.
func (m *Model) SetLive(self string, channels []resource.Channel) {
	open := key(m.Channel())
	if !m.live || self != m.self {
		open = ""
		m.drafts = map[string]string{}
		m.composer.SetValue("")
	}

	m.live, m.self, m.channels = true, self, channels
	m.cursor = 0
	for i, ch := range channels {
		if key(ch) == open {
			m.cursor = i
			break
		}
	}

	m.composer.Placeholder = m.placeholder()
	m.clampScroll()
}

// SetStatus puts a note under the channel list; empty clears it.
func (m *Model) SetStatus(status string) { m.status = status }

// Open opens the room with id, reporting whether it is listed.
func (m *Model) Open(id string) bool {
	for i, ch := range m.channels {
		if ch.ID == id {
			m.switchTo(i)
			return true
		}
	}

	return false
}

// SetSample goes back to the sample rooms, for when nobody is signed in.
func (m *Model) SetSample() {
	if !m.live {
		return
	}

	m.live, m.self, m.channels, m.cursor = false, sample.Self, sample.Channels(), 0
	m.drafts = map[string]string{}
	m.composer.SetValue("")
	m.composer.Placeholder = m.placeholder()
	m.StopComposing()
	m.scroll = 0
}

// key names a room across reloads: its ID, or its name for a sample room.
func key(ch resource.Channel) string {
	if ch.ID != "" {
		return ch.ID
	}

	return ch.Name
}

func (m Model) Composing() bool { return m.composing }

func (m Model) Draft() string { return m.composer.Value() }

func (m *Model) MoveUp()   { m.switchTo(m.cursor - 1) }
func (m *Model) MoveDown() { m.switchTo(m.cursor + 1) }

func (m *Model) switchTo(i int) {
	i = min(max(i, 0), len(m.channels)-1)
	if i == m.cursor || i < 0 {
		return
	}

	m.drafts[key(m.Channel())] = m.composer.Value()
	m.cursor = i
	m.channels[i].Unread = 0
	m.composer.SetValue(m.drafts[key(m.Channel())])
	m.composer.Placeholder = m.placeholder()
	m.scroll = 0
}

func (m Model) placeholder() string {
	if len(m.channels) == 0 {
		return "join a room first"
	}

	return "write to " + m.Channel().Name
}

// StartComposing focuses the composer, unless there is no room to write to.
func (m *Model) StartComposing() tea.Cmd {
	if len(m.channels) == 0 {
		return nil
	}

	m.composing = true
	m.channels[m.cursor].Unread = 0

	return m.composer.Focus()
}

func (m *Model) StopComposing() {
	m.composing = false
	m.composer.Blur()
}

func (m *Model) UpdateComposer(msg tea.Msg) tea.Cmd {
	var cmd tea.Cmd
	m.composer, cmd = m.composer.Update(msg)

	return cmd
}

// Outgoing is a message written in a live room, for the app to send.
type Outgoing struct {
	RoomID  string
	Message resource.Message
}

// Send takes what was written. A sample room keeps it at once; a live room's
// message is handed back to be sent, and appears once the app records it.
func (m *Model) Send() (Outgoing, bool) {
	body := strings.TrimSpace(m.composer.Value())
	if body == "" || len(m.channels) == 0 {
		return Outgoing{}, false
	}

	message := resource.Message{
		Time:   time.Now().Format("15:04"),
		Sender: m.self,
		Body:   body,
	}
	if rest, ok := strings.CutPrefix(body, emotePrefix); ok && strings.TrimSpace(rest) != "" {
		message.Body, message.Kind = strings.TrimSpace(rest), resource.Emote
	}

	ch := &m.channels[m.cursor]
	delete(m.drafts, key(*ch))
	m.composer.SetValue("")
	m.scroll = 0

	if m.live {
		return Outgoing{RoomID: ch.ID, Message: message}, true
	}

	ch.Messages = append(ch.Messages, message)
	ch.ReadBy = nil

	return Outgoing{}, false
}

func (m *Model) ScrollUp()     { m.scrollTo(m.scroll + m.timelineHeight()) }
func (m *Model) ScrollDown()   { m.scrollTo(m.scroll - m.timelineHeight()) }
func (m *Model) ScrollOldest() { m.scrollTo(m.maxScroll()) }
func (m *Model) ScrollNewest() { m.scrollTo(0) }

func (m *Model) scrollTo(n int) {
	m.scroll = n
	m.clampScroll()
}

func (m *Model) clampScroll() {
	m.scroll = min(max(m.scroll, 0), m.maxScroll())
}

func (m Model) maxScroll() int {
	return max(len(m.timeline(m.Channel(), m.conversationWidth()-2))-m.timelineHeight(), 0)
}

func (m Model) Summary() (resource.Row, bool) {
	ch := m.Channel()

	kind := "room"
	if ch.Direct {
		kind = "direct message"
	}

	encryption, emphasis := "no", resource.Held
	if ch.Encrypted {
		encryption, emphasis = "yes", resource.Done
	}

	return resource.Row{
		Cells: []string{ch.Name},
		Detail: []resource.Field{
			{Label: "Room", Value: ch.Name},
			{Label: "Kind", Value: kind},
			{Label: "Members", Value: itoa(len(ch.Members))},
			{Label: "Messages", Value: itoa(len(ch.Messages))},
			{Label: "Encrypted", Value: encryption, Emphasis: emphasis},
			{Label: "Threads", Value: itoa(threads(ch))},
			{Label: "Typing", Value: orNone(strings.Join(m.names(ch, ch.Typing), ", "))},
			{Label: "Read by", Value: orNone(strings.Join(m.names(ch, ch.ReadBy), ", "))},
		},
	}, true
}

func threads(ch resource.Channel) int {
	n := 0
	for _, msg := range ch.Messages {
		if msg.Replies > 0 {
			n++
		}
	}

	return n
}

func orNone(s string) string {
	if s == "" {
		return "nobody"
	}

	return s
}

func (m Model) showChannels() bool {
	return m.width-ChannelsWidth >= MinConversationWidth
}

func (m Model) conversationWidth() int {
	if m.showChannels() {
		return m.width - ChannelsWidth
	}

	return m.width
}

const conversationChrome = 2 + 4

func (m Model) timelineHeight() int {
	return max(m.height-conversationChrome, 1)
}
