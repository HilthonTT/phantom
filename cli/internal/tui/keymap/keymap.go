// Package keymap is every key the interface binds, and the help text for each.
//
// The bindings and the help menu are built from the same values, so a key
// cannot be rebound without the help following it.
package keymap

import "charm.land/bubbles/v2/key"

// KeyMap is the whole binding set.
type KeyMap struct {
	// Moving within a panel.
	Up       key.Binding
	Down     key.Binding
	PageUp   key.Binding
	PageDown key.Binding
	Top      key.Binding
	Bottom   key.Binding

	// Moving between panels.
	NextPanel  key.Binding
	PrevPanel  key.Binding
	OpenPanel  key.Binding
	ClosePanel key.Binding
	FocusNext  key.Binding
	FocusPrev  key.Binding

	// Acting on rows.
	Mark      key.Binding
	MarkAll   key.Binding
	ClearMark key.Binding
	Open      key.Binding
	Refresh   key.Binding

	// Opening things over the top of the layout.
	Filter key.Binding
	Prompt key.Binding
	Sort   key.Binding
	Help   key.Binding
	Cancel key.Binding

	// Leaving.
	Quit key.Binding
}

// Default is the binding set the interface ships with. The movement keys take
// both the arrows and their vi equivalents; everything else is a single letter
// so that the help menu stays readable.
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

// Entry is one line of the help menu: a heading, or a binding and what it
// does.
type Entry struct {
	// Heading is set on a section title, in which case Keys and Description
	// are empty.
	Heading string

	Keys        []string
	Description string
}

// Entries is the help menu's contents, in the order it lists them.
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
