package main

import (
	"fmt"
	"image"
	"image/color"
	"image/draw"
	"math"
	"os"
	"path/filepath"

	"golang.org/x/image/font"
	"golang.org/x/image/font/opentype"
	"golang.org/x/image/font/sfnt"
	"golang.org/x/image/math/fixed"
)

const (
	fontPx     = 15.0
	lineHeight = 1.3
	titlebar   = 36
	padding    = 14
	radius     = 12
)

// page is the colour around the window, the website's background, so a GIF
// without transparency still blends into the page.
var page = color.RGBA{0x0b, 0x0c, 0x12, 0xff}

type face struct {
	sfnt *sfnt.Font
	face font.Face
}

// Renderer draws grids as a terminal window.
type Renderer struct {
	// styles are regular, bold and italic; fallbacks are tried in order for
	// a character none of them has.
	styles    [3]face
	fallbacks []face

	cellW, cellH int
	ascent       int

	buf sfnt.Buffer
}

// LoadRenderer reads JetBrains Mono from dir, the fonts `make fonts`
// downloads, and DejaVu from the system for the symbols it lacks.
func LoadRenderer(dir string) (*Renderer, error) {
	load := func(path string) (face, error) {
		raw, err := os.ReadFile(path)
		if err != nil {
			return face{}, err
		}
		f, err := opentype.Parse(raw)
		if err != nil {
			return face{}, fmt.Errorf("%s: %w", path, err)
		}
		fc, err := opentype.NewFace(f, &opentype.FaceOptions{Size: fontPx, DPI: 72, Hinting: font.HintingFull})
		if err != nil {
			return face{}, fmt.Errorf("%s: %w", path, err)
		}
		return face{sfnt: f, face: fc}, nil
	}

	r := &Renderer{}
	for i, name := range []string{
		"JetBrainsMonoNerdFontMono-Regular.ttf",
		"JetBrainsMonoNerdFontMono-Bold.ttf",
		"JetBrainsMonoNerdFontMono-Italic.ttf",
	} {
		f, err := load(filepath.Join(dir, name))
		if err != nil {
			return nil, fmt.Errorf("%w (run `make fonts` first)", err)
		}
		r.styles[i] = f
	}

	for _, path := range []string{
		"/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
		"/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
	} {
		if f, err := load(path); err == nil {
			r.fallbacks = append(r.fallbacks, f)
		}
	}

	regular := r.styles[0].face
	adv, _ := regular.GlyphAdvance('M')
	r.cellW = adv.Round()
	r.cellH = int(math.Round(fontPx * lineHeight))

	m := regular.Metrics()
	content := (m.Ascent + m.Descent).Round()
	r.ascent = m.Ascent.Round() + (r.cellH-content)/2

	return r, nil
}

// Size is the image a cols by rows grid renders to.
func (r *Renderer) Size(cols, rows int) (int, int) {
	return cols*r.cellW + 2*padding, rows*r.cellH + titlebar + 2*padding
}

// Render draws the grid inside a window with a title bar.
func (r *Renderer) Render(grid Grid, title string) *image.RGBA {
	rows := len(grid)
	cols := 0
	if rows > 0 {
		cols = len(grid[0])
	}

	w, h := r.Size(cols, rows)
	img := image.NewRGBA(image.Rect(0, 0, w, h))
	draw.Draw(img, img.Bounds(), image.NewUniform(page), image.Point{}, draw.Src)

	window := color.RGBA{0x11, 0x11, 0x1b, 0xff}
	bar := color.RGBA{0x18, 0x18, 0x25, 0xff}
	roundedRect(img, img.Bounds(), radius, window)
	roundedTop(img, image.Rect(0, 0, w, titlebar), radius, bar)

	for i, c := range []color.RGBA{{0xf3, 0x8b, 0xa8, 0xff}, {0xf9, 0xe2, 0xaf, 0xff}, {0xa6, 0xe3, 0xa1, 0xff}} {
		disc(img, 22+i*20, titlebar/2, 6, c)
	}
	r.text(img, title, (w-len([]rune(title))*r.cellW)/2, (titlebar-r.cellH)/2+r.ascent, color.RGBA{0xa6, 0xad, 0xc8, 0xff})

	ox, oy := padding, titlebar+padding
	for y, row := range grid {
		for x, cell := range row {
			px, py := ox+x*r.cellW, oy+y*r.cellH
			cw := r.cellW
			if x+1 < len(row) && row[x+1].Cont {
				cw *= 2
			}
			fill(img, image.Rect(px, py, px+cw, py+r.cellH), cell.BG)
		}
	}

	for y, row := range grid {
		for x, cell := range row {
			if cell.Cont || cell.R == ' ' {
				continue
			}

			fg := cell.FG
			if cell.Faint {
				fg = blend(cell.FG, cell.BG, 0.55)
			}

			px, py := ox+x*r.cellW, oy+y*r.cellH
			if r.shape(img, cell.R, px, py, fg, cell.BG) {
				continue
			}
			r.glyph(img, cell, px, py+r.ascent, fg)
		}
	}

	return img
}

// glyph draws a character from the first font that has it.
func (r *Renderer) glyph(img *image.RGBA, cell Cell, x, baseline int, fg color.RGBA) {
	style := 0
	switch {
	case cell.Bold:
		style = 1
	case cell.Italic:
		style = 2
	}

	candidates := append([]face{r.styles[style], r.styles[0]}, r.fallbacks...)
	for _, f := range candidates {
		if idx, err := f.sfnt.GlyphIndex(&r.buf, cell.R); err == nil && idx != 0 {
			d := font.Drawer{Dst: img, Src: image.NewUniform(fg), Face: f.face, Dot: fixed.P(x, baseline)}
			d.DrawString(string(cell.R))
			return
		}
	}
}

func (r *Renderer) text(img *image.RGBA, s string, x, baseline int, fg color.RGBA) {
	d := font.Drawer{Dst: img, Src: image.NewUniform(fg), Face: r.styles[0].face, Dot: fixed.P(x, baseline)}
	d.DrawString(s)
}

// lines says which half-lines a box-drawing character has: up, down, left,
// right, and whether they are heavy.
var lines = map[rune][5]bool{
	'─': {false, false, true, true, false}, '━': {false, false, true, true, true},
	'│': {true, true, false, false, false}, '┃': {true, true, false, false, true},
	'┌': {false, true, false, true, false}, '┐': {false, true, true, false, false},
	'└': {true, false, false, true, false}, '┘': {true, false, true, false, false},
	'╭': {false, true, false, true, false}, '╮': {false, true, true, false, false},
	'╰': {true, false, false, true, false}, '╯': {true, false, true, false, false},
	'├': {true, true, false, true, false}, '┤': {true, true, true, false, false},
	'┬': {false, true, true, true, false}, '┴': {true, false, true, true, false},
	'┼': {true, true, true, true, false},
}

// shape draws box-drawing and block characters as shapes, so borders join
// at any line height; it reports whether the character was one of them.
func (r *Renderer) shape(img *image.RGBA, ch rune, x, y int, fg, bg color.RGBA) bool {
	w, h := r.cellW, r.cellH

	if l, ok := lines[ch]; ok {
		t := 1
		if l[4] {
			t = 2
		}
		cx, cy := x+w/2, y+h/2
		if l[0] {
			fill(img, image.Rect(cx, y, cx+t, cy+t), fg)
		}
		if l[1] {
			fill(img, image.Rect(cx, cy, cx+t, y+h), fg)
		}
		if l[2] {
			fill(img, image.Rect(x, cy, cx+t, cy+t), fg)
		}
		if l[3] {
			fill(img, image.Rect(cx, cy, x+w, cy+t), fg)
		}
		return true
	}

	switch ch {
	case '█':
		fill(img, image.Rect(x, y, x+w, y+h), fg)
	case '▓':
		fill(img, image.Rect(x, y, x+w, y+h), blend(fg, bg, 0.75))
	case '▒':
		fill(img, image.Rect(x, y, x+w, y+h), blend(fg, bg, 0.5))
	case '░':
		fill(img, image.Rect(x, y, x+w, y+h), blend(fg, bg, 0.25))
	case '⧗':
		// An hourglass, which few fonts carry: two triangles meeting at the
		// middle.
		hourglass(img, x, y, w, h, fg)
	case '▀':
		fill(img, image.Rect(x, y, x+w, y+h/2), fg)
	case '▄':
		fill(img, image.Rect(x, y+h/2, x+w, y+h), fg)
	default:
		return false
	}

	return true
}

func hourglass(img *image.RGBA, x, y, w, h int, c color.RGBA) {
	top, bottom := y+h/4, y+h*3/4
	mid := (top + bottom) / 2
	half := (w - 2) / 2
	cx := x + w/2

	for row := top; row <= bottom; row++ {
		d := mid - row
		if d < 0 {
			d = -d
		}
		span := half * d / max(mid-top, 1)
		fill(img, image.Rect(cx-span, row, cx+span+1, row+1), c)
	}
}

func fill(img *image.RGBA, r image.Rectangle, c color.RGBA) {
	draw.Draw(img, r, image.NewUniform(c), image.Point{}, draw.Src)
}

func blend(a, b color.RGBA, t float64) color.RGBA {
	mix := func(x, y uint8) uint8 { return uint8(float64(x)*t + float64(y)*(1-t)) }
	return color.RGBA{mix(a.R, b.R), mix(a.G, b.G), mix(a.B, b.B), 0xff}
}

func disc(img *image.RGBA, cx, cy, r int, c color.RGBA) {
	for y := -r; y <= r; y++ {
		for x := -r; x <= r; x++ {
			if x*x+y*y <= r*r {
				img.Set(cx+x, cy+y, c)
			}
		}
	}
}

// roundedRect fills r with its corners rounded off, leaving the page
// colour outside them.
func roundedRect(img *image.RGBA, r image.Rectangle, rad int, c color.RGBA) {
	for y := r.Min.Y; y < r.Max.Y; y++ {
		for x := r.Min.X; x < r.Max.X; x++ {
			if insideRounded(x, y, r, rad, true) {
				img.Set(x, y, c)
			}
		}
	}
}

// roundedTop is roundedRect with only the top corners rounded.
func roundedTop(img *image.RGBA, r image.Rectangle, rad int, c color.RGBA) {
	for y := r.Min.Y; y < r.Max.Y; y++ {
		for x := r.Min.X; x < r.Max.X; x++ {
			if insideRounded(x, y, r, rad, false) {
				img.Set(x, y, c)
			}
		}
	}
}

func insideRounded(x, y int, r image.Rectangle, rad int, bottom bool) bool {
	corner := func(cx, cy int) bool {
		dx, dy := x-cx, y-cy
		return dx*dx+dy*dy <= rad*rad
	}

	left, right := x < r.Min.X+rad, x >= r.Max.X-rad
	top, low := y < r.Min.Y+rad, y >= r.Max.Y-rad

	switch {
	case top && left:
		return corner(r.Min.X+rad, r.Min.Y+rad)
	case top && right:
		return corner(r.Max.X-rad-1, r.Min.Y+rad)
	case bottom && low && left:
		return corner(r.Min.X+rad, r.Max.Y-rad-1)
	case bottom && low && right:
		return corner(r.Max.X-rad-1, r.Max.Y-rad-1)
	}

	return true
}
