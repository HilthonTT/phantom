package session

import (
	"os"
	"path/filepath"
	"testing"

	"github.com/HilthonTT/phantom/cli/internal/client"
)

func TestSaveLoadForget(t *testing.T) {
	s := At(filepath.Join(t.TempDir(), "phantom", "sessions.json"))
	one := client.Session{UserID: "@a:one", DeviceID: "D1", AccessToken: "t1"}
	two := client.Session{UserID: "@b:two", DeviceID: "D2", AccessToken: "t2"}

	if _, ok, err := s.Load("http://one"); err != nil || ok {
		t.Fatalf("Load on no file = %v, %v; want nothing", ok, err)
	}

	if err := s.Save("http://one", one); err != nil {
		t.Fatal(err)
	}
	if err := s.Save("http://two", two); err != nil {
		t.Fatal(err)
	}

	if got, ok, _ := s.Load("http://one"); !ok || got != one {
		t.Errorf("Load one = %+v, %v", got, ok)
	}

	if err := s.Forget("http://one"); err != nil {
		t.Fatal(err)
	}
	if _, ok, _ := s.Load("http://one"); ok {
		t.Error("one survived Forget")
	}
	if got, ok, _ := s.Load("http://two"); !ok || got != two {
		t.Errorf("Forget one lost two: %+v, %v", got, ok)
	}
}

func TestFileIsOwnerOnly(t *testing.T) {
	s := At(filepath.Join(t.TempDir(), "sessions.json"))
	if err := s.Save("http://one", client.Session{AccessToken: "secret"}); err != nil {
		t.Fatal(err)
	}

	info, err := os.Stat(s.Path())
	if err != nil {
		t.Fatal(err)
	}
	if mode := info.Mode().Perm(); mode != 0o600 {
		t.Errorf("mode = %o, want 600", mode)
	}
}
