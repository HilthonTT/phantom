package chat

import (
	"hash/fnv"
	"image/color"
	"strconv"
	"strings"

	"charm.land/lipgloss/v2"
	"github.com/charmbracelet/x/ansi"

	"github.com/HilthonTT/phantom/cli/internal/tui/panel"
	"github.com/HilthonTT/phantom/cli/internal/tui/resource"
)

const (
	timeWidth   = 5
	senderWidth = 10
	gutter      = 1
)

func (m Model) Render(focused bool) string {
	if !m.showChannels() {
		return m.renderConversation(focused, m.width)
	}

	return lipgloss.JoinHorizontal(lipgloss.Top,
		m.renderChannels(focused),
		m.renderConversation(focused, m.conversationWidth()),
	)
}

func (m Model) renderChannels(focused bool) string {
	p := panel.New(m.theme.PanelConfig(ChannelsWidth, m.height, focused && !m.composing))
	p.SetTitle("Channels")
	if !m.live {
		p.SetTitle("Channels " + m.glyphs.Bullet + " sample")
	}

	p.AddLine("")

	direct := false
	p.AddLine(m.heading("ROOMS", p.ContentWidth()))
	if len(m.channels) == 0 {
		p.AddLine(m.theme.Faint.Render("   no rooms yet"))
	}
	for i, ch := range m.channels {
		if p.Remaining() < 1 {
			break
		}
		if ch.Direct && !direct {
			direct = true
			p.AddLine("")
			p.AddLine(m.heading("DIRECT", p.ContentWidth()))
		}
		p.AddLine(m.channelEntry(ch, i == m.cursor, focused && !m.composing, p.ContentWidth()))
	}

	switch unread := m.unread(); {
	case m.status != "":
		p.SetInfo(m.status)
	case unread > 0:
		p.SetInfo(itoa(unread) + " unread")
	}

	return p.Render()
}

func (m Model) heading(label string, width int) string {
	const lead = 2

	label = " " + label + " "
	rule := max(width-panel.Width(label)-lead-2, 0)

	return m.theme.Faint.Render(" "+strings.Repeat(m.glyphs.Divider, lead)) +
		m.theme.Heading.Render(label) +
		m.theme.Faint.Render(strings.Repeat(m.glyphs.Divider, rule))
}

func (m Model) channelEntry(ch resource.Channel, open, focused bool, width int) string {
	cursor := "  "
	if open && focused {
		cursor = " " + m.glyphs.Cursor
	}

	glyph := m.glyphs.Room
	if ch.Direct {
		glyph = m.glyphs.User
	}

	badge := ""
	if ch.Unread > 0 {
		badge = " " + itoa(ch.Unread) + " "
	}

	style := m.theme.Muted
	switch {
	case open:
		style = m.theme.RowSelected
	case ch.Unread > 0:
		style = m.theme.Text.Bold(true)
	}

	nameWidth := max(width-panel.Width(cursor)-panel.Width(badge)-1, 1)
	name := panel.Pad(" "+glyph+" "+panel.Truncate(ch.Name, nameWidth-3), nameWidth)

	return m.theme.Cursor.Render(cursor) + style.Render(name) + m.badge(badge, open)
}

func (m Model) badge(s string, open bool) string {
	if s == "" {
		return ""
	}

	bg := m.theme.Palette.Surface
	if open {
		bg = m.theme.Palette.Sunken
	}

	return lipgloss.NewStyle().Foreground(m.theme.Palette.Accent).Background(bg).Bold(true).Render(s)
}

func (m Model) unread() int {
	n := 0
	for _, ch := range m.channels {
		n += ch.Unread
	}

	return n
}

func (m Model) renderConversation(focused bool, width int) string {
	ch := m.Channel()
	p := panel.New(m.theme.PanelConfig(width, m.height, focused))
	p.SetTitle(m.title(ch))

	p.AddLine(m.theme.Muted.Render(" " + panel.Truncate(ch.Topic, max(p.ContentWidth()-2, 1))))
	p.AddDivider()

	lines := m.timeline(ch, p.ContentWidth())
	height := m.timelineHeight()

	end := max(len(lines)-m.scroll, 0)
	start := max(end-height, 0)
	for range height - (end - start) {
		p.AddLine("")
	}
	p.AddLines(lines[start:end]...)

	p.AddDivider()
	p.AddLine(m.composer.View())

	p.SetInfo(m.info(ch)...)

	return p.Render()
}

func (m Model) title(ch resource.Channel) string {
	title := ch.Name
	if ch.Encrypted {
		title += " " + m.glyphs.Bullet + " encrypted"
	}
	if !m.showChannels() {
		title += " " + m.glyphs.Arrow + " " + itoa(m.cursor+1) + "/" + itoa(len(m.channels))
	}

	return title
}

func (m Model) info(ch resource.Channel) []string {
	first := itoa(len(ch.Members)) + " members"
	if typing := m.typing(ch); typing != "" {
		first = typing
	}

	switch {
	case m.scroll > 0:
		return []string{first, "↑ " + itoa(m.scroll) + " lines"}
	case m.composing:
		return []string{first, "enter sends"}
	default:
		return []string{first, "enter to write"}
	}
}

func (m Model) typing(ch resource.Channel) string {
	switch names := m.names(ch, ch.Typing); len(names) {
	case 0:
		return ""
	case 1:
		return names[0] + " is typing…"
	case 2:
		return names[0] + " and " + names[1] + " are typing…"
	default:
		return itoa(len(names)) + " people are typing…"
	}
}

// names turns member IDs into display names, leaving out the reader's own.
func (m Model) names(ch resource.Channel, ids []string) []string {
	names := make([]string, 0, len(ids))
	for _, id := range ids {
		if id != m.self {
			names = append(names, m.displayName(ch, id))
		}
	}

	return names
}

// displayName is a member's name in the room, or the localpart of an ID the
// room has no member for.
func (m Model) displayName(ch resource.Channel, id string) string {
	for _, member := range ch.Members {
		if member.ID == id {
			return member.Name
		}
	}

	return localpart(id)
}

func (m Model) timeline(ch resource.Channel, width int) []string {
	var lines []string

	previous := ""
	for _, msg := range ch.Messages {
		lines = append(lines, m.message(ch, msg, msg.Sender == previous && msg.Kind == resource.Text, width)...)

		previous = ""
		if msg.Kind == resource.Text {
			previous = msg.Sender
		}
	}

	if seen := m.names(ch, ch.ReadBy); len(seen) > 0 {
		receipt := panel.Truncate(m.glyphs.Done+" seen by "+strings.Join(seen, ", "), max(width-2, 1))
		lines = append(lines, m.theme.Faint.Render(
			strings.Repeat(" ", max(width-panel.Width(receipt)-1, 0))+receipt))
	}

	return lines
}

func (m Model) message(ch resource.Channel, msg resource.Message, continued bool, width int) []string {
	lead := " " + panel.Pad(msg.Time, timeWidth) + " "
	if continued {
		lead = strings.Repeat(" ", panel.Width(lead))
	}

	switch msg.Kind {
	case resource.Membership:
		return m.wrap(m.theme.Faint.Render(lead),
			m.theme.Faint, m.glyphs.Arrow+" "+m.displayName(ch, msg.Sender)+" "+msg.Body, width)

	case resource.Emote:
		return m.wrap(m.theme.Faint.Render(lead),
			m.senderStyle(msg.Sender).Bold(false).Italic(true),
			"* "+m.displayName(ch, msg.Sender)+" "+msg.Body, width)
	}

	name := ""
	if !continued {
		name = m.displayName(ch, msg.Sender)
	}

	prefix := m.theme.Faint.Render(lead) +
		m.senderStyle(msg.Sender).Render(panel.Pad(panel.Truncate(name, senderWidth), senderWidth)) +
		m.theme.Text.Render(strings.Repeat(" ", gutter))

	if msg.Redacted {
		return m.wrap(prefix, m.theme.Faint.Italic(true), "message deleted", width)
	}

	body := m.theme.Text
	if msg.Kind == resource.Notice {
		body = m.theme.Muted.Italic(true)
	}

	lines := m.wrap(prefix, body, msg.Body, width)
	switch {
	case msg.Failed:
		lines = m.suffix(lines, " (not sent)", panel.Width(prefix), width)
	case msg.Pending:
		lines = m.suffix(lines, " (sending…)", panel.Width(prefix), width)
	case msg.Edited:
		lines = m.suffix(lines, " (edited)", panel.Width(prefix), width)
	}

	return append(lines, m.annotations(msg, panel.Width(prefix), width)...)
}

// suffix appends a faint marker to the last line, or below it when the line
// is already full.
func (m Model) suffix(lines []string, marker string, indent, width int) []string {
	last := len(lines) - 1
	if panel.Width(lines[last])+panel.Width(marker) < width {
		lines[last] += m.theme.Faint.Render(marker)
		return lines
	}

	return append(lines, m.theme.Text.Render(strings.Repeat(" ", indent))+
		m.theme.Faint.Render(strings.TrimSpace(marker)))
}

// annotations are the lines under a message's body: its reactions, then a
// summary of the thread it starts.
func (m Model) annotations(msg resource.Message, indent, width int) []string {
	pad := m.theme.Text.Render(strings.Repeat(" ", indent))
	room := max(width-indent-1, 1)

	var lines []string
	if len(msg.Reactions) > 0 {
		chips := make([]string, 0, len(msg.Reactions))
		for _, r := range msg.Reactions {
			style := m.theme.Muted
			if r.Mine {
				style = m.theme.Cursor
			}
			chips = append(chips, style.Render(r.Key+" "+itoa(r.Count)))
		}
		lines = append(lines, pad+panel.Truncate(strings.Join(chips, m.theme.Text.Render("  ")), room))
	}

	if msg.Replies > 0 {
		replies := itoa(msg.Replies) + " replies"
		if msg.Replies == 1 {
			replies = "1 reply"
		}
		lines = append(lines, pad+m.theme.Faint.Render(panel.Truncate("↳ "+replies+" in thread", room)))
	}

	return lines
}

func (m Model) wrap(prefix string, style lipgloss.Style, body string, width int) []string {
	indent := panel.Width(prefix)
	room := max(width-indent-1, 1)

	parts := strings.Split(ansi.Wrap(body, room, ""), "\n")
	lines := make([]string, 0, len(parts))
	for i, part := range parts {
		lead := prefix
		if i > 0 {
			lead = m.theme.Text.Render(strings.Repeat(" ", indent))
		}
		lines = append(lines, lead+style.Render(strings.TrimRight(part, " ")))
	}

	return lines
}

func (m Model) senderStyle(id string) lipgloss.Style {
	if id == m.self {
		return m.theme.Title
	}

	// Not the reds: those are your own name (Title) and danger.
	palette := []color.Color{
		m.theme.Palette.Info,
		m.theme.Palette.Success,
		m.theme.Palette.Warning,
		m.theme.Palette.Heading,
	}

	h := fnv.New32a()
	_, _ = h.Write([]byte(id))

	return lipgloss.NewStyle().
		Foreground(palette[h.Sum32()%uint32(len(palette))]).
		Background(m.theme.Palette.Surface).
		Bold(true)
}

func (m Model) RenderMembers(width, height int) string {
	ch := m.Channel()
	p := panel.New(m.theme.PanelConfig(width, height, false))
	p.SetTitle("Members")

	p.AddLine("")

	var admins, members []resource.Member
	online := 0
	for _, member := range ch.Members {
		if member.Presence != resource.Offline {
			online++
		}
		if member.Admin {
			admins = append(admins, member)
		} else {
			members = append(members, member)
		}
	}

	m.memberGroup(p, "ADMINS", admins)
	if len(admins) > 0 && len(members) > 0 && p.Remaining() > 0 {
		p.AddLine("")
	}
	m.memberGroup(p, "MEMBERS", members)

	p.SetInfo(itoa(online) + " online")

	return p.Render()
}

func (m Model) memberGroup(p *panel.Panel, label string, members []resource.Member) {
	if len(members) == 0 || p.Remaining() < 1 {
		return
	}

	p.AddLine(m.heading(label, p.ContentWidth()))
	for _, member := range members {
		if p.Remaining() < 1 {
			return
		}
		p.AddLine(m.member(member, p.ContentWidth()))
	}
}

func (m Model) member(member resource.Member, width int) string {
	presence := m.theme.Faint
	switch member.Presence {
	case resource.Online:
		presence = m.theme.StateDone
	case resource.Idle:
		presence = m.theme.StateHeld
	}

	name := member.Name
	room := max(width-5, 1)
	id := ""
	if rest := room - panel.Width(name) - 1; rest > 4 {
		id = " " + panel.Truncate(member.ID, rest)
	}

	style := m.theme.Text
	if member.Presence == resource.Offline {
		style = m.theme.Muted
	}

	return presence.Render("  "+m.glyphs.Marked+" ") +
		style.Render(panel.Truncate(name, room)) +
		m.theme.Faint.Render(id)
}

func localpart(id string) string {
	id = strings.TrimPrefix(id, "@")
	if name, _, ok := strings.Cut(id, ":"); ok {
		return name
	}

	return id
}

func itoa(n int) string { return strconv.Itoa(n) }
