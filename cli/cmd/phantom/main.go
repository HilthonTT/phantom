package main

import (
	"fmt"
	"os"

	"github.com/HilthonTT/phantom/cli/internal/tui"
)

func main() {
	if err := tui.Run(); err != nil {
		fmt.Fprintln(os.Stderr, "phantom:", err)
		os.Exit(1)
	}
}
