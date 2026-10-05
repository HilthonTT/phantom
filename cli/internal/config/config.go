// Package config reads the CLI's settings from its flags and environment.
package config

import (
	"flag"
	"fmt"
	"io"
	"os"

	"github.com/HilthonTT/phantom/cli/internal/client"
)

// ServerEnv names the variable that sets the server when --server is not
// given.
const ServerEnv = "PHANTOM_SERVER"

type Config struct {
	// Server is the base URL of the phantom-server to connect to.
	Server string
}

// Load parses args, the command line without the program name. A --server
// flag wins over $PHANTOM_SERVER, which wins over the server's own default
// listener.
func Load(args []string, stderr io.Writer) (Config, error) {
	fallback := client.DefaultURL
	if env := os.Getenv(ServerEnv); env != "" {
		fallback = env
	}

	fs := flag.NewFlagSet("phantom", flag.ContinueOnError)
	fs.SetOutput(stderr)

	var c Config
	fs.StringVar(&c.Server, "server", fallback,
		"base URL of the phantom-server to manage (or $"+ServerEnv+")")

	if err := fs.Parse(args); err != nil {
		return Config{}, err
	}
	if fs.NArg() > 0 {
		return Config{}, fmt.Errorf("unexpected argument %q", fs.Arg(0))
	}

	return c, nil
}
