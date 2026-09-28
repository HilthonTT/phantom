package app

import (
	"strings"
	"testing"

	tea "charm.land/bubbletea/v2"
	"charm.land/lipgloss/v2"
)

func openChat(t *testing.T, width, height int) Model {
	t.Helper()

	m := sized(t, width, height)
	m.focus = focusSidebar
	m = arrow(t, m, tea.KeyEnter)

	if !m.chatOpen {
		t.Fatal("opening the first sidebar section did not open the chat")
	}
	if m.focus != focusWorkspace {
		t.Fatalf("focus = %d, want the chat", m.focus)
	}

	return m
}

func TestChatFillsTheTerminalExactly(t *testing.T) {
	sizes := []struct{ width, height int }{
		{80, 24},
		{96, 26},
		{110, 30},
		{140, 40},
		{240, 60},
	}

	for _, s := range sizes {
		m := openChat(t, s.width, s.height)

		for _, composing := range []bool{false, true} {
			if composing {
				m = arrow(t, m, tea.KeyEnter)
			}
			out := m.render()

			if got := lipgloss.Width(out); got != s.width {
				t.Errorf("at %dx%d (composing %v): width = %d, want %d",
					s.width, s.height, composing, got, s.width)
			}
			if got := lipgloss.Height(out); got != s.height {
				t.Errorf("at %dx%d (composing %v): height = %d, want %d",
					s.width, s.height, composing, got, s.height)
			}
		}
	}
}

func TestComposingSendsAMessage(t *testing.T) {
	m := openChat(t, 140, 40)
	before := len(m.chat.Channel().Messages)

	m = arrow(t, m, tea.KeyEnter)
	if !m.chat.Composing() {
		t.Fatal("enter did not start composing")
	}

	m = press(t, m, "q", "u", "i", "t")
	if m.quitting {
		t.Fatal("typing `q` while composing quit phantom")
	}

	m = arrow(t, m, tea.KeyEnter)

	messages := m.chat.Channel().Messages
	if len(messages) != before+1 {
		t.Fatalf("have %d messages, want %d", len(messages), before+1)
	}
	if got := messages[len(messages)-1].Body; got != "quit" {
		t.Errorf("sent %q, want \"quit\"", got)
	}
	if m.chat.Draft() != "" {
		t.Errorf("the composer kept %q after sending", m.chat.Draft())
	}
	if !strings.Contains(m.render(), "quit") {
		t.Error("the sent message is not on screen")
	}

	m = arrow(t, m, tea.KeyEscape)
	if m.chat.Composing() {
		t.Error("esc did not stop composing")
	}
}

func TestEmptyMessagesAreNotSent(t *testing.T) {
	m := openChat(t, 140, 40)
	before := len(m.chat.Channel().Messages)

	m = arrow(t, m, tea.KeyEnter)
	m = press(t, m, " ")
	m = arrow(t, m, tea.KeyEnter)

	if got := len(m.chat.Channel().Messages); got != before {
		t.Errorf("have %d messages after sending blanks, want %d", got, before)
	}
}

func TestSwitchingChannelsKeepsTheDraft(t *testing.T) {
	m := openChat(t, 140, 40)
	first := m.chat.Channel().Name

	m = arrow(t, m, tea.KeyEnter)
	m = press(t, m, "h", "i")
	m = arrow(t, m, tea.KeyEscape)

	m = press(t, m, "j")
	if m.chat.Channel().Name == first {
		t.Fatal("`j` did not switch channels")
	}
	if m.chat.Draft() != "" {
		t.Errorf("the new channel shows the draft %q", m.chat.Draft())
	}
	if m.chat.Channel().Unread != 0 {
		t.Error("opening a channel left it unread")
	}

	m = press(t, m, "k")
	if got := m.chat.Draft(); got != "hi" {
		t.Errorf("draft = %q after switching back, want \"hi\"", got)
	}
}

func TestOpeningAnotherSectionLeavesTheChat(t *testing.T) {
	m := openChat(t, 140, 40)

	m.focus = focusSidebar
	m = arrow(t, m, tea.KeyDown)
	m = arrow(t, m, tea.KeyEnter)

	if m.chatOpen {
		t.Error("opening another section left the chat open")
	}
}
