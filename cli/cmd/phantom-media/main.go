// Command phantom-media records the GIFs and screenshots on the
// documentation website: it starts a scratch phantom-server, fills it with a
// small community, drives the real console against it, and renders what the
// console shows to images.
//
//	go run ./cmd/phantom-media -server-bin ../target/debug/phantom-server
package main

import (
	"flag"
	"fmt"
	"os"
	"path/filepath"

	"github.com/HilthonTT/phantom/cli/internal/client"
	"github.com/HilthonTT/phantom/cli/internal/session"
	"github.com/HilthonTT/phantom/cli/internal/tui/app"
)

func main() {
	bin := flag.String("server-bin", "../target/release/phantom-server", "the phantom-server binary to record against")
	fonts := flag.String("fonts", "../target/fonts", "where `make fonts` put JetBrains Mono")
	out := flag.String("out", "../website/public/media", "where the GIFs and screenshots go")
	port := flag.Int("port", 18090, "the scratch server's port")
	cols := flag.Int("cols", 132, "terminal width")
	rows := flag.Int("rows", 36, "terminal height")
	flag.Parse()

	if err := record(*bin, *fonts, *out, *port, *cols, *rows); err != nil {
		fmt.Fprintln(os.Stderr, "phantom-media:", err)
		os.Exit(1)
	}
}

func record(bin, fonts, out string, port, cols, rows int) error {
	r, err := LoadRenderer(fonts)
	if err != nil {
		return err
	}
	if err := os.MkdirAll(out, 0o755); err != nil {
		return err
	}

	fmt.Println("starting a scratch phantom-server…")
	server, err := StartServer(bin, port)
	if err != nil {
		return err
	}
	defer server.Stop()

	fmt.Println("seeding it…")
	seeded, err := Seed(server.URL)
	if err != nil {
		return err
	}

	c, err := client.New(server.URL)
	if err != nil {
		return err
	}

	dir, err := os.MkdirTemp("", "phantom-media-sessions-")
	if err != nil {
		return err
	}
	defer func() { _ = os.RemoveAll(dir) }()

	console := app.New(c, session.At(filepath.Join(dir, "sessions.json")))
	rec := &Recording{d: NewDriver(console, cols, rows, r, "phantom — "+serverName), out: out, seed: seeded}

	fmt.Println("recording…")
	for _, scene := range []func() error{rec.Overview, rec.Chat, rec.Actions, rec.Operations, rec.Screenshots} {
		if err := scene(); err != nil {
			return err
		}
	}

	fmt.Println("done:", out)
	return nil
}
