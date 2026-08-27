package theme

// Glyphs are the single-width symbols the interface draws with.
//
// These are plain Unicode rather than Nerd Font private-use codepoints, so a
// terminal without a patched font still renders them. [ASCIIGlyphs] is the
// fallback for a terminal that cannot manage even that.
type Glyphs struct {
	Cursor  string // points at the row under the cursor
	Marked  string // a row the user has marked for an operation
	Bullet  string // an item in a list of values
	Divider string // fills a sidebar section rule
	Arrow   string // separates a panel's title from what it has selected

	Server    string
	Room      string
	User      string
	Federated string
	Media     string
	Task      string
	Log       string
	Config    string

	Running string
	Done    string
	Failed  string
	Held    string

	ProgressFull  rune
	ProgressEmpty rune
}

// UnicodeGlyphs is the default set.
func UnicodeGlyphs() Glyphs {
	return Glyphs{
		Cursor:  "▸",
		Marked:  "●",
		Bullet:  "·",
		Divider: "─",
		Arrow:   "›",

		Server:    "◆",
		Room:      "▣",
		User:      "◍",
		Federated: "◈",
		Media:     "▤",
		Task:      "⧗",
		Log:       "≡",
		Config:    "⚙",

		Running: "◐",
		Done:    "✔",
		Failed:  "✖",
		Held:    "⏸",

		ProgressFull:  '█',
		ProgressEmpty: '░',
	}
}

// ASCIIGlyphs is the fallback set for terminals that cannot render
// [UnicodeGlyphs].
func ASCIIGlyphs() Glyphs {
	return Glyphs{
		Cursor:  ">",
		Marked:  "*",
		Bullet:  "-",
		Divider: "-",
		Arrow:   ">",

		Server:    "#",
		Room:      "#",
		User:      "@",
		Federated: "~",
		Media:     "%",
		Task:      "!",
		Log:       "=",
		Config:    "+",

		Running: "~",
		Done:    "+",
		Failed:  "x",
		Held:    "=",

		ProgressFull:  '#',
		ProgressEmpty: '.',
	}
}
