package main

import (
	"errors"
	"flag"
	"fmt"
	"os"

	"github.com/HilthonTT/phantom/cli/internal/client"
	"github.com/HilthonTT/phantom/cli/internal/config"
	"github.com/HilthonTT/phantom/cli/internal/session"
	"github.com/HilthonTT/phantom/cli/internal/tui"
)

func main() {
	cfg, err := config.Load(os.Args[1:], os.Stderr)
	if errors.Is(err, flag.ErrHelp) {
		return
	}
	if err != nil {
		fail(err)
	}

	c, err := client.New(cfg.Server)
	if err != nil {
		fail(err)
	}

	store, err := session.Default()
	if err != nil {
		fail(err)
	}

	if err := tui.Run(c, store); err != nil {
		fail(err)
	}
}

func fail(err error) {
	fmt.Fprintln(os.Stderr, "phantom:", err)
	os.Exit(1)
}
