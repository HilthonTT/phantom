package theme

import (
	"image/color"

	"charm.land/bubbles/v2/textinput"
)

// Input returns a text input styled for this theme, sitting on the given
// background — [Palette.Surface] for one inside a panel, [Palette.Raised] for
// one inside a modal.
//
// The prompt is the sigil printed ahead of what is typed: `/` for a filter,
// `:` for the command prompt.
func (t Theme) Input(prompt, placeholder string, background color.Color) textinput.Model {
	in := textinput.New()
	in.Prompt = prompt
	in.Placeholder = placeholder

	styles := in.Styles()
	for _, state := range []*textinput.StyleState{&styles.Focused, &styles.Blurred} {
		state.Text = state.Text.Foreground(t.Palette.Text).Background(background)
		state.Placeholder = state.Placeholder.Foreground(t.Palette.Faint).Background(background)
		state.Prompt = state.Prompt.Foreground(t.Palette.Accent).Background(background)
		state.Suggestion = state.Suggestion.Foreground(t.Palette.Faint).Background(background)
	}
	styles.Cursor.Color = t.Palette.Accent
	in.SetStyles(styles)

	return in
}
