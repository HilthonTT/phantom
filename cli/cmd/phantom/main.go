// Command phantom is the CLI for administering a phantom homeserver.
//
// Run with no arguments it opens the admin console; see
// [github.com/HilthonTT/phantom/cli/internal/tui].
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
