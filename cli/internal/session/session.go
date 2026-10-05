// Package session keeps the console's logins between runs, one per server,
// in a file only its owner can read.
package session

import (
	"encoding/json"
	"errors"
	"fmt"
	"io/fs"
	"os"
	"path/filepath"

	"github.com/HilthonTT/phantom/cli/internal/client"
)

// Store is the sessions file.
type Store struct {
	path string
}

// Default is the sessions file under the user's config directory.
func Default() (Store, error) {
	dir, err := os.UserConfigDir()
	if err != nil {
		return Store{}, fmt.Errorf("finding the config directory: %w", err)
	}

	return At(filepath.Join(dir, "phantom", "sessions.json")), nil
}

func At(path string) Store { return Store{path: path} }

func (s Store) Path() string { return s.path }

// Load returns the session saved for server, if any.
func (s Store) Load(server string) (client.Session, bool, error) {
	all, err := s.read()
	if err != nil {
		return client.Session{}, false, err
	}

	saved, ok := all[server]

	return saved, ok, nil
}

// Save keeps session as the one for server.
func (s Store) Save(server string, session client.Session) error {
	all, err := s.read()
	if err != nil {
		return err
	}

	all[server] = session

	return s.write(all)
}

// Forget drops the session saved for server.
func (s Store) Forget(server string) error {
	all, err := s.read()
	if err != nil {
		return err
	}
	if _, ok := all[server]; !ok {
		return nil
	}

	delete(all, server)

	return s.write(all)
}

func (s Store) read() (map[string]client.Session, error) {
	all := map[string]client.Session{}

	raw, err := os.ReadFile(s.path)
	if errors.Is(err, fs.ErrNotExist) {
		return all, nil
	}
	if err != nil {
		return nil, err
	}

	if err := json.Unmarshal(raw, &all); err != nil {
		return nil, fmt.Errorf("%s: %w", s.path, err)
	}

	return all, nil
}

// write replaces the file through a temporary one, so a crash never leaves
// it half written, and creates both owner-only since they hold tokens.
func (s Store) write(all map[string]client.Session) error {
	raw, err := json.MarshalIndent(all, "", "  ")
	if err != nil {
		return err
	}

	if err := os.MkdirAll(filepath.Dir(s.path), 0o700); err != nil {
		return err
	}

	tmp, err := os.CreateTemp(filepath.Dir(s.path), ".sessions-*.json")
	if err != nil {
		return err
	}
	defer func() { _ = os.Remove(tmp.Name()) }()

	if err := tmp.Chmod(0o600); err != nil {
		_ = tmp.Close()
		return err
	}
	if _, err := tmp.Write(raw); err != nil {
		_ = tmp.Close()
		return err
	}
	if err := tmp.Close(); err != nil {
		return err
	}

	return os.Rename(tmp.Name(), s.path)
}
