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

func (m Model) Channel() resource.Channel { return m.channels[m.cursor] }

func (m Model) Composing() bool { return m.composing }

func (m Model) Draft() string { return m.composer.Value() }

func (m *Model) MoveUp()   { m.switchTo(m.cursor - 1) }
func (m *Model) MoveDown() { m.switchTo(m.cursor + 1) }

func (m *Model) switchTo(i int) {
	i = min(max(i, 0), len(m.channels)-1)
	if i == m.cursor {
		return
	}

	m.channels[m.cursor].Draft = m.composer.Value()
	m.cursor = i
	m.channels[i].Unread = 0
	m.composer.SetValue(m.channels[i].Draft)
	m.composer.Placeholder = m.placeholder()
	m.scroll = 0
}

func (m Model) placeholder() string {
	return "write to " + m.Channel().Name
}

func (m *Model) StartComposing() tea.Cmd {
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

func (m *Model) Send() {
	body := strings.TrimSpace(m.composer.Value())
	if body == "" {
		return
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
	ch.Messages = append(ch.Messages, message)
	ch.ReadBy = nil
	ch.Draft = ""
	m.composer.SetValue("")
	m.scroll = 0
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
