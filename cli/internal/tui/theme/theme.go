// Package theme holds the colour palette the TUI is drawn with and the
// lipgloss styles derived from it.
//
// Every colour the interface uses is named here once. A component asks for a
// style by what it is drawing — [Theme.PanelTitle], [Theme.RowSelected] — and
// never names a hex value of its own, so re-theming is a change to this file
// alone.
package theme

import (
	"image/color"

	"charm.land/lipgloss/v2"
)

// Palette is the set of colours a theme is built from. The names are roles
// rather than hues, so a light theme can fill the same fields.
type Palette struct {
	// Backgrounds, from the outermost surface inwards.
	Canvas  color.Color // behind everything
	Surface color.Color // inside a panel
	Raised  color.Color // inside a modal
	Sunken  color.Color // a selected row

	// Foregrounds.
	Text   color.Color // ordinary content
	Muted  color.Color // labels, secondary content
	Faint  color.Color // dividers, disabled content
	Accent color.Color // the cursor and anything it points at

	// Panel borders, idle and focused.
	Border       color.Color
	BorderActive color.Color

	// Status colours, used for task state and connection state.
	Success color.Color
	Warning color.Color
	Danger  color.Color
	Info    color.Color

	// Section headings in the sidebar and the help menu.
	Heading color.Color
	// A hotkey as printed in the help menu and the prompt.
	Hotkey color.Color
}

// Theme is a palette together with the styles built from it. Construct one
// with [New]; the zero value renders nothing legible.
type Theme struct {
	Palette Palette

	// Surfaces.
	Canvas lipgloss.Style
	Panel  lipgloss.Style
	Modal  lipgloss.Style

	// Text roles.
	Text    lipgloss.Style
	Muted   lipgloss.Style
	Faint   lipgloss.Style
	Heading lipgloss.Style
	Title   lipgloss.Style

	// The cursor, and the row it is on.
	Cursor      lipgloss.Style
	RowSelected lipgloss.Style
	RowMarked   lipgloss.Style

	// Column headers in a resource listing.
	ColumnHeader lipgloss.Style

	// Task and connection state.
	StateRunning lipgloss.Style
	StateDone    lipgloss.Style
	StateFailed  lipgloss.Style
	StateHeld    lipgloss.Style

	// Modal furniture.
	ModalTitle   lipgloss.Style
	ModalConfirm lipgloss.Style
	ModalCancel  lipgloss.Style
	ModalHint    lipgloss.Style

	// Help menu and prompt.
	Hotkey       lipgloss.Style
	PromptSigil  lipgloss.Style
	PromptOK     lipgloss.Style
	PromptFailed lipgloss.Style
}

// Mocha is the default palette: Catppuccin Mocha, the same family superfile
// ships as its default.
func Mocha() Palette {
	return Palette{
		Canvas:  lipgloss.Color("#11111b"),
		Surface: lipgloss.Color("#1e1e2e"),
		Raised:  lipgloss.Color("#181825"),
		Sunken:  lipgloss.Color("#313244"),

		Text:   lipgloss.Color("#cdd6f4"),
		Muted:  lipgloss.Color("#a6adc8"),
		Faint:  lipgloss.Color("#585b70"),
		Accent: lipgloss.Color("#f5c2e7"),

		Border:       lipgloss.Color("#45475a"),
		BorderActive: lipgloss.Color("#89b4fa"),

		Success: lipgloss.Color("#a6e3a1"),
		Warning: lipgloss.Color("#f9e2af"),
		Danger:  lipgloss.Color("#f38ba8"),
		Info:    lipgloss.Color("#89dceb"),

		Heading: lipgloss.Color("#cba6f7"),
		Hotkey:  lipgloss.Color("#fab387"),
	}
}

// New builds the styles for a palette.
func New(p Palette) Theme {
	on := func(fg, bg color.Color) lipgloss.Style {
		return lipgloss.NewStyle().Foreground(fg).Background(bg)
	}

	return Theme{
		Palette: p,

		Canvas: on(p.Text, p.Canvas),
		Panel:  on(p.Text, p.Surface),
		Modal:  on(p.Text, p.Raised),

		Text:    on(p.Text, p.Surface),
		Muted:   on(p.Muted, p.Surface),
		Faint:   on(p.Faint, p.Surface),
		Heading: on(p.Heading, p.Surface).Bold(true),
		Title:   on(p.BorderActive, p.Surface).Bold(true),

		Cursor:      on(p.Accent, p.Surface),
		RowSelected: on(p.Text, p.Sunken),
		RowMarked:   on(p.Accent, p.Sunken),

		ColumnHeader: on(p.Faint, p.Surface).Bold(true),

		StateRunning: on(p.Info, p.Surface),
		StateDone:    on(p.Success, p.Surface),
		StateFailed:  on(p.Danger, p.Surface),
		StateHeld:    on(p.Warning, p.Surface),

		ModalTitle:   on(p.Heading, p.Raised).Bold(true),
		ModalConfirm: on(p.Raised, p.Success).Bold(true),
		ModalCancel:  on(p.Raised, p.Danger).Bold(true),
		ModalHint:    on(p.Muted, p.Raised),

		Hotkey:       on(p.Hotkey, p.Raised),
		PromptSigil:  on(p.Accent, p.Raised).Bold(true),
		PromptOK:     on(p.Success, p.Raised),
		PromptFailed: on(p.Danger, p.Raised),
	}
}

// Default is [New] applied to [Mocha].
func Default() Theme { return New(Mocha()) }
