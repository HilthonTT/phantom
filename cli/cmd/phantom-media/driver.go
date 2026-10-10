package main

import (
	"time"

	tea "charm.land/bubbletea/v2"

	"github.com/HilthonTT/phantom/cli/internal/tui/live"
	"github.com/HilthonTT/phantom/cli/internal/tui/resource"
)

// Driver runs the console headless: it plays its commands, feeds their
// messages back, presses keys, and records frames of what it shows.
type Driver struct {
	m    tea.Model
	msgs chan tea.Msg

	width, height int
	r             *Renderer
	title         string

	frames []Frame

	// sidebar is where the sidebar's cursor is, which the driver tracks so
	// it can walk it to a section.
	sidebar int
}

func NewDriver(m tea.Model, width, height int, r *Renderer, title string) *Driver {
	d := &Driver{m: m, msgs: make(chan tea.Msg, 1024), width: width, height: height, r: r, title: title}
	d.feed(tea.WindowSizeMsg{Width: width, Height: height})
	d.run(m.Init())

	return d
}

// run plays a command on its own goroutine, as Bubble Tea would, its
// messages coming back through msgs.
func (d *Driver) run(cmd tea.Cmd) {
	if cmd == nil {
		return
	}

	go func() {
		msg := cmd()
		if batch, ok := msg.(tea.BatchMsg); ok {
			for _, c := range batch {
				d.run(c)
			}
			return
		}
		if msg != nil {
			d.msgs <- msg
		}
	}()
}

func (d *Driver) feed(msg tea.Msg) {
	next, cmd := d.m.Update(msg)
	d.m = next
	d.run(cmd)
}

// Settle lets the console work for a while: answers from the server
// arrive and are applied. The probe timer is dropped, so a recording is not
// interrupted by a refresh.
func (d *Driver) Settle(dur time.Duration) {
	deadline := time.After(dur)
	for {
		select {
		case msg := <-d.msgs:
			if _, tick := msg.(live.TickMsg); tick {
				continue
			}
			d.feed(msg)
		case <-deadline:
			return
		}
	}
}

// Snap records what the console shows now, held for hold.
func (d *Driver) Snap(hold time.Duration) {
	frame := Parse(d.m.View().Content, d.width, d.height)
	d.frames = append(d.frames, Frame{Image: d.r.Render(frame, d.title), Delay: int(hold / (10 * time.Millisecond))})
}

// Press presses a key and lets the console react.
func (d *Driver) Press(code rune, text string) {
	d.feed(tea.KeyPressMsg{Code: code, Text: text})
	d.Settle(60 * time.Millisecond)
}

// Key presses a key without text, such as enter or an arrow.
func (d *Driver) Key(code rune) { d.Press(code, "") }

// Type types text a character at a time, recording a frame per character.
func (d *Driver) Type(text string, perChar time.Duration) {
	for _, r := range text {
		d.feed(tea.KeyPressMsg{Code: r, Text: string(r)})
		d.Settle(20 * time.Millisecond)
		d.Snap(perChar)
	}
}

// Open walks the sidebar to a section and opens it, recording a frame at
// each step.
func (d *Driver) Open(section resource.Section, step time.Duration) {
	target := 0
	for i, s := range resource.Sections() {
		if s == section {
			target = i
		}
	}

	d.Press('H', "H")
	for d.sidebar != target {
		if d.sidebar < target {
			d.Key(tea.KeyDown)
			d.sidebar++
		} else {
			d.Key(tea.KeyUp)
			d.sidebar--
		}
		d.Snap(step)
	}

	d.Key(tea.KeyEnter)
	d.Settle(400 * time.Millisecond)
}

// Take hands over the frames recorded so far and starts afresh.
func (d *Driver) Take() []Frame {
	frames := d.frames
	d.frames = nil

	return frames
}
