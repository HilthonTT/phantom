package keymap

import "charm.land/bubbles/v2/key"

type KeyMap struct {
	Up       key.Binding
	Down     key.Binding
	PageUp   key.Binding
	PageDown key.Binding
	Top      key.Binding
	Bottom   key.Binding

	NextPanel  key.Binding
	PrevPanel  key.Binding
	OpenPanel  key.Binding
	ClosePanel key.Binding
	FocusNext  key.Binding
	FocusPrev  key.Binding

	Mark      key.Binding
	MarkAll   key.Binding
	ClearMark key.Binding
	Open      key.Binding
	Refresh   key.Binding

	Compose key.Binding
	Send    key.Binding

	// The Logs section's own keys.
	LogFollow key.Binding
	LogPause  key.Binding
	LogLevel  key.Binding
	LogTarget key.Binding
	LogSearch key.Binding
	LogDebug  key.Binding

	// Inspect opens the room inspector on a Rooms row, and Older loads the
	// next page where a listing is read a page at a time.
	Inspect  key.Binding
	Older    key.Binding
	NextView key.Binding
	PrevView key.Binding
	Expand   key.Binding

	Filter key.Binding
	Prompt key.Binding
	Sort   key.Binding
	Help   key.Binding
	Cancel key.Binding

	Quit      key.Binding
	ForceQuit key.Binding
}

func Default() KeyMap {
	return KeyMap{
		Up:       binding("move up", "up", "k"),
		Down:     binding("move down", "down", "j"),
		PageUp:   binding("page up", "pgup", "ctrl+u"),
		PageDown: binding("page down", "pgdown", "ctrl+d"),
		Top:      binding("jump to first row", "home", "g"),
		Bottom:   binding("jump to last row", "end", "G"),

		NextPanel:  binding("focus the next panel", "tab"),
		PrevPanel:  binding("focus the previous panel", "shift+tab"),
		OpenPanel:  binding("open another panel", "n"),
		ClosePanel: binding("close this panel", "w"),
		FocusNext:  binding("move focus clockwise", "ctrl+right", "L"),
		FocusPrev:  binding("move focus anticlockwise", "ctrl+left", "H"),

		Mark:      binding("mark the row under the cursor", "space"),
		MarkAll:   binding("mark every row", "a"),
		ClearMark: binding("clear all marks", "A"),
		Open:      binding("open what the cursor is on", "enter"),
		Refresh:   binding("reload the listing", "r"),

		Compose: binding("write in the open room", "enter"),
		Send:    binding("send what you wrote", "enter"),

		LogFollow: binding("follow the log as it grows, or stop", "f"),
		LogPause:  binding("pause or resume following", "p"),
		LogLevel:  binding("step the lowest level shown", "v"),
		LogTarget: binding("show one target module's lines", "t"),
		LogSearch: binding("search the lines' text", "m"),
		LogDebug:  binding("keep the server's debug lines, or stop", "d"),

		Inspect:  binding("inspect the room under the cursor", "i"),
		Older:    binding("load older events or audit records", "o"),
		NextView: binding("the inspector's next view", "tab"),
		PrevView: binding("the inspector's previous view", "shift+tab"),
		Expand:   binding("show or hide the event's JSON", "enter", "space"),

		Filter: binding("filter what is listed", "/"),
		Prompt: binding("open the command prompt", ":"),
		Sort:   binding("change the sort order", "s"),
		Help:   binding("show this help", "?"),
		Cancel: binding("dismiss what is open, or cancel a task", "esc"),

		Quit:      binding("quit phantom, after asking", "q"),
		ForceQuit: binding("quit phantom at once", "ctrl+c"),
	}
}

func binding(help string, keys ...string) key.Binding {
	return key.NewBinding(key.WithKeys(keys...), key.WithHelp(keys[0], help))
}

type Entry struct {
	Heading string

	Keys        []string
	Description string
}

func (k KeyMap) Entries() []Entry {
	section := func(title string, bindings ...key.Binding) []Entry {
		entries := []Entry{{Heading: title}}
		for _, b := range bindings {
			entries = append(entries, Entry{
				Keys:        b.Keys(),
				Description: b.Help().Desc,
			})
		}
		return entries
	}

	var help []Entry
	help = append(help, section("Movement", k.Up, k.Down, k.PageUp, k.PageDown, k.Top, k.Bottom)...)
	help = append(help, section("Panels",
		k.NextPanel, k.PrevPanel, k.OpenPanel, k.ClosePanel, k.FocusNext, k.FocusPrev)...)
	help = append(help, section("Rows", k.Mark, k.MarkAll, k.ClearMark, k.Open, k.Refresh)...)
	help = append(help, section("Chat", k.Compose, k.Send, k.PageUp, k.PageDown, k.Cancel)...)
	help = append(help, section("Logs", k.LogFollow, k.LogPause, k.LogLevel, k.LogTarget, k.LogSearch, k.LogDebug)...)
	help = append(help, section("Room inspector",
		k.Inspect, k.NextView, k.PrevView, k.Expand, k.Older, k.Refresh, k.Cancel)...)
	help = append(help, section("Overlays", k.Filter, k.Prompt, k.Sort, k.Help, k.Cancel)...)
	help = append(help, section("Session", k.Quit, k.ForceQuit)...)

	return help
}
