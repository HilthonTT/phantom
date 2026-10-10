package main

import (
	"image/color"
	"strconv"
	"strings"

	"github.com/mattn/go-runewidth"
)

// Cell is one terminal cell: a character and how it is drawn. The second
// cell of a double-width character is a Cont cell with no character of its
// own.
type Cell struct {
	R      rune
	FG, BG color.RGBA
	Bold   bool
	Italic bool
	Faint  bool
	Cont   bool
}

// Grid is a frame as a terminal would show it, rows of cells.
type Grid [][]Cell

var (
	defaultFG = color.RGBA{0xcc, 0xcc, 0xcc, 0xff}
	defaultBG = color.RGBA{0x0a, 0x0a, 0x0a, 0xff}
)

// Parse lays out a frame of styled text, as a Bubble Tea view renders it,
// on a width by height grid. It reads the SGR sequences lipgloss writes:
// truecolor and 256-colour foregrounds and backgrounds, bold, faint, italic
// and resets; anything else is skipped.
func Parse(frame string, width, height int) Grid {
	grid := make(Grid, height)
	for y := range grid {
		grid[y] = make([]Cell, width)
		for x := range grid[y] {
			grid[y][x] = Cell{R: ' ', FG: defaultFG, BG: defaultBG}
		}
	}

	st := Cell{FG: defaultFG, BG: defaultBG}
	x, y := 0, 0

	for i := 0; i < len(frame); {
		switch c := frame[i]; {
		case c == 0x1b && i+1 < len(frame) && frame[i+1] == '[':
			end := i + 2
			for end < len(frame) && (frame[end] < 0x40 || frame[end] > 0x7e) {
				end++
			}
			if end < len(frame) && frame[end] == 'm' {
				st = applySGR(st, frame[i+2:end])
			}
			i = end + 1

		case c == 0x1b && i+1 < len(frame) && frame[i+1] == ']':
			// An OSC sequence, such as a hyperlink, ends at BEL or ST.
			end := i + 2
			for end < len(frame) && frame[end] != 0x07 && !isST(frame, end) {
				end++
			}
			if end < len(frame) && frame[end] == 0x1b {
				end++
			}
			i = end + 1

		case c == '\n':
			x, y = 0, y+1
			i++

		case c == '\r':
			x = 0
			i++

		default:
			r, size := decodeRune(frame[i:])
			i += size

			w := runewidth.RuneWidth(r)
			if w == 0 || y >= height {
				continue
			}

			if x+w <= width {
				cell := st
				cell.R = r
				grid[y][x] = cell
				if w == 2 {
					cont := st
					cont.R, cont.Cont = ' ', true
					grid[y][x+1] = cont
				}
			}
			x += w
		}
	}

	return grid
}

// isST is whether the string terminator, ESC \, starts at i.
func isST(s string, i int) bool {
	return s[i] == 0x1b && i+1 < len(s) && s[i+1] == '\\'
}

func decodeRune(s string) (rune, int) {
	for _, r := range s {
		return r, len(string(r))
	}

	return ' ', 1
}

func applySGR(st Cell, params string) Cell {
	if params == "" {
		params = "0"
	}

	codes := strings.Split(params, ";")
	for i := 0; i < len(codes); i++ {
		n, _ := strconv.Atoi(codes[i])
		switch {
		case n == 0:
			st = Cell{FG: defaultFG, BG: defaultBG}
		case n == 1:
			st.Bold = true
		case n == 2:
			st.Faint = true
		case n == 3:
			st.Italic = true
		case n == 22:
			st.Bold, st.Faint = false, false
		case n == 23:
			st.Italic = false
		case n == 39:
			st.FG = defaultFG
		case n == 49:
			st.BG = defaultBG
		case n == 38 || n == 48:
			c, used := extendedColor(codes[i+1:])
			i += used
			if used == 0 {
				continue
			}
			if n == 38 {
				st.FG = c
			} else {
				st.BG = c
			}
		case n >= 30 && n <= 37:
			st.FG = ansi16[n-30]
		case n >= 90 && n <= 97:
			st.FG = ansi16[n-90+8]
		case n >= 40 && n <= 47:
			st.BG = ansi16[n-40]
		case n >= 100 && n <= 107:
			st.BG = ansi16[n-100+8]
		}
	}

	return st
}

// extendedColor reads the arguments after a 38 or 48: "2;r;g;b" or "5;n",
// reporting how many codes it used.
func extendedColor(codes []string) (color.RGBA, int) {
	if len(codes) == 0 {
		return color.RGBA{}, 0
	}

	switch codes[0] {
	case "2":
		if len(codes) < 4 {
			return color.RGBA{}, len(codes)
		}
		r, _ := strconv.Atoi(codes[1])
		g, _ := strconv.Atoi(codes[2])
		b, _ := strconv.Atoi(codes[3])
		return color.RGBA{uint8(r), uint8(g), uint8(b), 0xff}, 4

	case "5":
		if len(codes) < 2 {
			return color.RGBA{}, len(codes)
		}
		n, _ := strconv.Atoi(codes[1])
		return xterm256(n), 2
	}

	return color.RGBA{}, 0
}

var ansi16 = [16]color.RGBA{
	{0x2c, 0x26, 0x27, 0xff}, {0xf0, 0x19, 0x3d, 0xff}, {0x2e, 0xcc, 0x70, 0xff}, {0xf9, 0xb8, 0x1f, 0xff},
	{0x6b, 0x9b, 0xf7, 0xff}, {0xff, 0x4d, 0x67, 0xff}, {0x5f, 0xd4, 0xc4, 0xff}, {0xcc, 0xcc, 0xcc, 0xff},
	{0x4a, 0x43, 0x44, 0xff}, {0xff, 0x6b, 0x6b, 0xff}, {0x5c, 0xe0, 0x93, 0xff}, {0xfb, 0xcb, 0x55, 0xff},
	{0x93, 0xb6, 0xfa, 0xff}, {0xff, 0x8a, 0x9b, 0xff}, {0x8e, 0xe3, 0xd7, 0xff}, {0xf7, 0xf7, 0xf7, 0xff},
}

func xterm256(n int) color.RGBA {
	switch {
	case n < 16:
		return ansi16[n]
	case n < 232:
		n -= 16
		level := func(v int) uint8 {
			if v == 0 {
				return 0
			}
			return uint8(55 + v*40)
		}
		return color.RGBA{level(n / 36), level(n / 6 % 6), level(n % 6), 0xff}
	default:
		v := uint8(8 + (n-232)*10)
		return color.RGBA{v, v, v, 0xff}
	}
}
