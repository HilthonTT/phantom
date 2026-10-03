package theme

type Glyphs struct {
	Cursor  string
	Marked  string
	Bullet  string
	Divider string
	Arrow   string

	Server    string
	Service   string
	API       string
	Room      string
	User      string
	Device    string
	Token     string
	Federated string
	Bridge    string
	Media     string
	Task      string
	Report    string
	Log       string
	Config    string
	Chat      string

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
		API:       "⇄",
		Room:      "▣",
		User:      "◍",
		Device:    "▭",
		Token:     "◇",
		Federated: "◈",
		Bridge:    "⋈",
		Media:     "▤",
		Task:      "⧗",
		Report:    "⚑",
		Log:       "≡",
		Config:    "⚙",
		Chat:      "◧",

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
		API:       "/",
		Room:      "#",
		User:      "@",
		Device:    "d",
		Token:     "$",
		Federated: "~",
		Bridge:    "^",
		Media:     "%",
		Task:      "!",
		Report:    "?",
		Log:       "=",
		Config:    "+",
		Chat:      "\"",

		Running: "~",
		Done:    "+",
		Failed:  "x",
		Held:    "=",

		ProgressFull:  '#',
		ProgressEmpty: '.',
	}
}
