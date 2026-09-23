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

	Filter key.Binding
	Prompt key.Binding
	Sort   key.Binding
	Help   key.Binding
	Cancel key.Binding

	Quit key.Binding
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

		Filter: binding("filter what is listed", "/"),
		Prompt: binding("open the command prompt", ":"),
		Sort:   binding("change the sort order", "s"),
		Help:   binding("show this help", "?"),
		Cancel: binding("dismiss what is open, or cancel a task", "esc"),

		Quit: binding("quit phantom", "q", "ctrl+c"),
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
	help = append(help, section("Overlays", k.Filter, k.Prompt, k.Sort, k.Help, k.Cancel)...)
	help = append(help, section("Session", k.Quit)...)

	return help
}
