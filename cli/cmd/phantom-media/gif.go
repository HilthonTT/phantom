package main

import (
	"image"
	"image/color"
	"image/gif"
	"image/png"
	"os"
	"sort"
)

// Frame is one image of a recording and how long it stays up.
type Frame struct {
	Image *image.RGBA
	Delay int // hundredths of a second
}

// WriteGIF encodes frames with one palette for the whole recording, so text
// keeps its colour from frame to frame, and stores each frame after the
// first as just the rectangle that changed.
func WriteGIF(path string, frames []Frame) error {
	pal := paletteOf(frames)
	lookup := map[color.RGBA]uint8{}

	out := &gif.GIF{LoopCount: 0}
	var prev *image.RGBA

	for _, f := range frames {
		rect := f.Image.Bounds()
		if prev != nil {
			changed, ok := diff(prev, f.Image)
			if !ok {
				// Nothing changed: hold the last frame longer instead.
				out.Delay[len(out.Delay)-1] += f.Delay
				continue
			}
			rect = changed
		}

		p := image.NewPaletted(rect, pal)
		for y := rect.Min.Y; y < rect.Max.Y; y++ {
			for x := rect.Min.X; x < rect.Max.X; x++ {
				c := f.Image.RGBAAt(x, y)
				i, ok := lookup[c]
				if !ok {
					i = uint8(pal.Index(c))
					lookup[c] = i
				}
				p.SetColorIndex(x, y, i)
			}
		}

		out.Image = append(out.Image, p)
		out.Delay = append(out.Delay, f.Delay)
		out.Disposal = append(out.Disposal, gif.DisposalNone)
		prev = f.Image
	}

	file, err := os.Create(path)
	if err != nil {
		return err
	}
	defer func() { _ = file.Close() }()

	return gif.EncodeAll(file, out)
}

func WritePNG(path string, img image.Image) error {
	file, err := os.Create(path)
	if err != nil {
		return err
	}
	defer func() { _ = file.Close() }()

	return (&png.Encoder{CompressionLevel: png.BestCompression}).Encode(file, img)
}

// diff is the smallest rectangle holding every pixel that differs.
func diff(a, b *image.RGBA) (image.Rectangle, bool) {
	bounds := b.Bounds()
	minX, minY, maxX, maxY := bounds.Max.X, bounds.Max.Y, -1, -1

	for y := bounds.Min.Y; y < bounds.Max.Y; y++ {
		for x := bounds.Min.X; x < bounds.Max.X; x++ {
			if a.RGBAAt(x, y) != b.RGBAAt(x, y) {
				minX, minY = min(minX, x), min(minY, y)
				maxX, maxY = max(maxX, x), max(maxY, y)
			}
		}
	}

	if maxX < 0 {
		return image.Rectangle{}, false
	}

	return image.Rect(minX, minY, maxX+1, maxY+1), true
}

type weighted struct {
	c color.RGBA
	n int
}

// paletteOf picks 256 colours for the frames by median cut, weighting each
// colour by how often it appears, so the large flat areas of the interface
// keep their exact colour and only glyph edges are approximated.
func paletteOf(frames []Frame) color.Palette {
	counts := map[color.RGBA]int{}
	for _, f := range frames {
		b := f.Image.Bounds()
		for y := b.Min.Y; y < b.Max.Y; y++ {
			for x := b.Min.X; x < b.Max.X; x++ {
				counts[f.Image.RGBAAt(x, y)]++
			}
		}
	}

	all := make([]weighted, 0, len(counts))
	for c, n := range counts {
		all = append(all, weighted{c, n})
	}

	if len(all) <= 256 {
		pal := make(color.Palette, len(all))
		for i, w := range all {
			pal[i] = w.c
		}
		return pal
	}

	boxes := [][]weighted{all}
	for len(boxes) < 256 {
		// Split the box with the widest channel range that can be split.
		best, bestRange, bestCh := -1, -1, 0
		for i, box := range boxes {
			if len(box) < 2 {
				continue
			}
			ch, rng := widest(box)
			if rng > bestRange {
				best, bestRange, bestCh = i, rng, ch
			}
		}
		if best < 0 {
			break
		}

		box := boxes[best]
		sort.Slice(box, func(i, j int) bool { return channel(box[i].c, bestCh) < channel(box[j].c, bestCh) })

		total := 0
		for _, w := range box {
			total += w.n
		}
		half, acc, cut := total/2, 0, 1
		for i, w := range box {
			acc += w.n
			if acc >= half {
				cut = max(1, min(i+1, len(box)-1))
				break
			}
		}

		boxes[best] = box[:cut]
		boxes = append(boxes, box[cut:])
	}

	pal := make(color.Palette, 0, len(boxes))
	for _, box := range boxes {
		var r, g, b, n int
		for _, w := range box {
			r += int(w.c.R) * w.n
			g += int(w.c.G) * w.n
			b += int(w.c.B) * w.n
			n += w.n
		}
		pal = append(pal, color.RGBA{uint8(r / n), uint8(g / n), uint8(b / n), 0xff})
	}

	return pal
}

func widest(box []weighted) (int, int) {
	lo := [3]int{255, 255, 255}
	hi := [3]int{}
	for _, w := range box {
		for ch := range 3 {
			v := channel(w.c, ch)
			lo[ch], hi[ch] = min(lo[ch], v), max(hi[ch], v)
		}
	}

	best := 0
	for ch := 1; ch < 3; ch++ {
		if hi[ch]-lo[ch] > hi[best]-lo[best] {
			best = ch
		}
	}

	return best, hi[best] - lo[best]
}

func channel(c color.RGBA, ch int) int {
	switch ch {
	case 0:
		return int(c.R)
	case 1:
		return int(c.G)
	default:
		return int(c.B)
	}
}
