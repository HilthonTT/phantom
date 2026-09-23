package theme

import (
	"image/color"

	"charm.land/lipgloss/v2"
)

type Palette struct {
	Canvas  color.Color
	Surface color.Color
	Raised  color.Color
	Sunken  color.Color

	Text   color.Color
	Muted  color.Color
	Faint  color.Color
	Accent color.Color

	Border       color.Color
	BorderActive color.Color

	Success color.Color
	Warning color.Color
	Danger  color.Color
	Info    color.Color

	Heading color.Color

	Hotkey color.Color
}

type Theme struct {
	Palette Palette

	Canvas lipgloss.Style
	Panel  lipgloss.Style
	Modal  lipgloss.Style

	Text    lipgloss.Style
	Muted   lipgloss.Style
	Faint   lipgloss.Style
	Heading lipgloss.Style
	Title   lipgloss.Style

	Cursor      lipgloss.Style
	RowSelected lipgloss.Style
	RowMarked   lipgloss.Style

	ColumnHeader lipgloss.Style

	StateRunning lipgloss.Style
	StateDone    lipgloss.Style
	StateFailed  lipgloss.Style
	StateHeld    lipgloss.Style

	ModalTitle   lipgloss.Style
	ModalConfirm lipgloss.Style
	ModalCancel  lipgloss.Style
	ModalHint    lipgloss.Style

	Hotkey       lipgloss.Style
	PromptSigil  lipgloss.Style
	PromptOK     lipgloss.Style
	PromptFailed lipgloss.Style
}

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

func Default() Theme { return New(Mocha()) }
