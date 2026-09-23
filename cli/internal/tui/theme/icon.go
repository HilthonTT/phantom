package theme

type Glyphs struct {
	Cursor  string
	Marked  string
	Bullet  string
	Divider string
	Arrow   string

	Server    string
	Service   string
	Room      string
	User      string
	Token     string
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

func UnicodeGlyphs() Glyphs {
	return Glyphs{
		Cursor:  "▸",
		Marked:  "●",
		Bullet:  "·",
		Divider: "─",
		Arrow:   "›",

		Server:    "◆",
		Service:   "◫",
		Room:      "▣",
		User:      "◍",
		Token:     "◇",
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

func ASCIIGlyphs() Glyphs {
	return Glyphs{
		Cursor:  ">",
		Marked:  "*",
		Bullet:  "-",
		Divider: "-",
		Arrow:   ">",

		Server:    "#",
		Service:   "&",
		Room:      "#",
		User:      "@",
		Token:     "$",
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
