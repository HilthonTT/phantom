package main

import (
	"fmt"
	"path/filepath"
	"time"

	tea "charm.land/bubbletea/v2"

	"github.com/HilthonTT/phantom/cli/internal/tui/resource"
)

const (
	beat  = 350 * time.Millisecond
	pause = 2200 * time.Millisecond
)

// Recording plays the scenes against a signed-in console and writes their
// GIFs and screenshots into out.
type Recording struct {
	d    *Driver
	out  string
	seed Seeded
}

func (r *Recording) gif(name string) error {
	frames := r.d.Take()
	fmt.Printf("  %s: %d frames\n", name, len(frames))

	return WriteGIF(filepath.Join(r.out, name), frames)
}

func (r *Recording) png(name string) error {
	r.d.Snap(0)
	frames := r.d.Take()

	return WritePNG(filepath.Join(r.out, name), frames[len(frames)-1].Image)
}

// command types a command at the prompt and runs it.
func (r *Recording) command(line string) {
	r.d.Press(':', ":")
	r.d.Snap(beat)
	r.d.Type(line, 70*time.Millisecond)
	r.d.Key(tea.KeyEnter)
	r.d.Settle(1500 * time.Millisecond)
}

// Overview signs in, then walks the main sections.
func (r *Recording) Overview() error {
	d := r.d

	d.Settle(1500 * time.Millisecond)
	d.Snap(pause)
	if err := r.png("login.png"); err != nil {
		return err
	}

	d.Type("ada", 110*time.Millisecond)
	d.Key(tea.KeyTab)
	d.Type(password, 35*time.Millisecond)
	d.Key(tea.KeyEnter)
	d.Settle(3 * time.Second)
	d.Snap(pause)

	for range 5 {
		d.Key(tea.KeyDown)
		d.Snap(beat)
	}

	for _, s := range []resource.Section{resource.Users, resource.Rooms, resource.Services, resource.Logs} {
		d.Open(s, 120*time.Millisecond)
		d.Snap(pause)
	}

	return r.gif("overview.gif")
}

// Chat opens the rooms, watches a member type, and sends a message.
func (r *Recording) Chat() error {
	d := r.d

	r.seed.Typing()
	d.Open(resource.Chat, 120*time.Millisecond)
	d.Settle(1500 * time.Millisecond)
	d.Snap(pause)
	if err := r.png("chat.png"); err != nil {
		return err
	}

	d.Key(tea.KeyEnter)
	d.Snap(beat)
	d.Type("deploying 0.1.1 tonight, back in ten minutes", 55*time.Millisecond)
	d.Key(tea.KeyEnter)
	d.Settle(1500 * time.Millisecond)
	d.Snap(pause)

	d.Key(tea.KeyEscape)
	for range 3 {
		d.Key(tea.KeyDown)
		d.Settle(300 * time.Millisecond)
		d.Snap(1200 * time.Millisecond)
	}

	return r.gif("chat.gif")
}

// Actions makes a user an admin from the row menu, then creates a token from
// the prompt.
func (r *Recording) Actions() error {
	d := r.d

	r.command("user alan")
	d.Snap(1200 * time.Millisecond)

	d.Key(tea.KeyEnter)
	d.Snap(pause / 2)
	d.Key(tea.KeyDown)
	d.Snap(beat * 2)
	d.Key(tea.KeyEnter)
	d.Snap(pause / 2)
	d.Key(tea.KeyTab)
	d.Snap(beat * 2)
	d.Key(tea.KeyEnter)
	d.Settle(2 * time.Second)
	d.Snap(pause)
	d.Key(tea.KeyEnter)
	d.Settle(500 * time.Millisecond)
	d.Snap(pause)

	r.command("token 25 30 spring-meetup")
	d.Snap(pause)
	d.Key(tea.KeyEnter)
	d.Open(resource.Tokens, 120*time.Millisecond)
	d.Settle(time.Second)
	d.Snap(pause)

	return r.gif("actions.gif")
}

// Operations dismisses a report, sorts the media store, and looks at the
// tasks and services.
func (r *Recording) Operations() error {
	d := r.d

	d.Open(resource.Reports, 120*time.Millisecond)
	d.Snap(pause)
	d.Key(tea.KeyEnter)
	d.Snap(pause / 2)
	d.Key(tea.KeyEnter)
	d.Snap(pause / 2)
	d.Key(tea.KeyTab)
	d.Key(tea.KeyEnter)
	d.Settle(1500 * time.Millisecond)
	d.Snap(pause)
	d.Key(tea.KeyEnter)
	d.Settle(500 * time.Millisecond)
	d.Snap(pause / 2)

	d.Open(resource.Media, 120*time.Millisecond)
	d.Snap(pause)
	for range 3 {
		d.Press('s', "s")
		d.Snap(beat * 3)
	}

	d.Open(resource.Tasks, 120*time.Millisecond)
	d.Snap(pause)

	return r.gif("operations.gif")
}

// Screenshots takes a still of each section for the docs.
func (r *Recording) Screenshots() error {
	shots := []struct {
		section resource.Section
		name    string
	}{
		{resource.Overview, "overview.png"},
		{resource.Users, "users.png"},
		{resource.Devices, "devices.png"},
		{resource.Tokens, "tokens.png"},
		{resource.Rooms, "rooms.png"},
		{resource.Services, "services.png"},
		{resource.Federation, "federation.png"},
		{resource.Media, "media.png"},
		{resource.Tasks, "tasks.png"},
		{resource.Reports, "reports.png"},
		{resource.Logs, "logs.png"},
		{resource.Settings, "settings.png"},
	}

	for _, s := range shots {
		r.d.Open(s.section, 0)
		r.d.Settle(time.Second)
		r.d.Take()
		if err := r.png(s.name); err != nil {
			return err
		}
	}

	r.d.Press('?', "?")
	r.d.Settle(300 * time.Millisecond)
	if err := r.png("help.png"); err != nil {
		return err
	}
	r.d.Key(tea.KeyEscape)

	return nil
}
