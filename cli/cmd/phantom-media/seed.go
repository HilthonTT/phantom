package main

import (
	"bytes"
	"crypto/rand"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"os"
	"os/exec"
	"path/filepath"
	"time"
)

const (
	serverName = "phantom.chat"
	regToken   = "welcome-aboard"
	password   = "correct horse battery staple"
)

// Server is a scratch phantom-server for the recording, with its own
// database in a temporary directory.
type Server struct {
	URL string

	cmd *exec.Cmd
	dir string
}

func StartServer(bin string, port int) (*Server, error) {
	dir, err := os.MkdirTemp("", "phantom-media-")
	if err != nil {
		return nil, err
	}

	config := fmt.Sprintf(`[global]
server_name = %q
database_path = %q
database_backup_path = %q
port = %d
allow_registration = true
registration_token = %q
`, serverName, filepath.Join(dir, "db"), filepath.Join(dir, "backups"), port, regToken)

	path := filepath.Join(dir, "phantom.toml")
	if err := os.WriteFile(path, []byte(config), 0o600); err != nil {
		return nil, err
	}

	log, err := os.Create(filepath.Join(dir, "server.log"))
	if err != nil {
		return nil, err
	}

	cmd := exec.Command(bin, "-c", path)
	cmd.Stdout, cmd.Stderr = log, log
	if err := cmd.Start(); err != nil {
		return nil, err
	}

	s := &Server{URL: fmt.Sprintf("http://localhost:%d", port), cmd: cmd, dir: dir}
	for range 60 {
		if resp, err := http.Get(s.URL + "/_phantom/server_version"); err == nil {
			_ = resp.Body.Close()
			return s, nil
		}
		time.Sleep(500 * time.Millisecond)
	}

	s.Stop()
	return nil, fmt.Errorf("phantom-server did not start; see %s", log.Name())
}

func (s *Server) Stop() {
	if s.cmd.Process != nil {
		_ = s.cmd.Process.Signal(os.Interrupt)
		done := make(chan struct{})
		go func() { _ = s.cmd.Wait(); close(done) }()
		select {
		case <-done:
		case <-time.After(15 * time.Second):
			_ = s.cmd.Process.Kill()
		}
	}
	_ = os.RemoveAll(s.dir)
}

// account is a signed-in user of the scratch server.
type account struct {
	base  string
	id    string
	token string
}

func (a account) call(method, path string, in, out any) error {
	var body io.Reader
	if in != nil {
		raw, err := json.Marshal(in)
		if err != nil {
			return err
		}
		body = bytes.NewReader(raw)
	}

	req, err := http.NewRequest(method, a.base+path, body)
	if err != nil {
		return err
	}
	if a.token != "" {
		req.Header.Set("Authorization", "Bearer "+a.token)
	}
	req.Header.Set("Content-Type", "application/json")

	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		return err
	}
	defer func() { _ = resp.Body.Close() }()

	raw, _ := io.ReadAll(resp.Body)
	if resp.StatusCode != http.StatusOK {
		return fmt.Errorf("%s %s: %d %s", method, path, resp.StatusCode, raw)
	}
	if out != nil {
		return json.Unmarshal(raw, out)
	}

	return nil
}

func (a account) upload(name, contentType string, size int) error {
	data := make([]byte, size)
	_, _ = rand.Read(data)

	req, err := http.NewRequest(http.MethodPost, a.base+"/_matrix/media/v3/upload?filename="+url.QueryEscape(name), bytes.NewReader(data))
	if err != nil {
		return err
	}
	req.Header.Set("Authorization", "Bearer "+a.token)
	req.Header.Set("Content-Type", contentType)

	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		return err
	}
	_ = resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		return fmt.Errorf("upload %s: %d", name, resp.StatusCode)
	}

	return nil
}

// register makes an account through the token-gated registration flow,
// signed in on a device with the given name.
func register(base, user, display, device string) (account, error) {
	a := account{base: base}
	body := map[string]any{"username": user, "password": password, "initial_device_display_name": device}

	// The first request is answered 401 with the session the second one
	// completes.
	var flow struct {
		Session string `json:"session"`
	}
	raw, _ := json.Marshal(body)
	resp, err := http.Post(base+"/_matrix/client/v3/register", "application/json", bytes.NewReader(raw))
	if err != nil {
		return a, err
	}
	_ = json.NewDecoder(resp.Body).Decode(&flow)
	_ = resp.Body.Close()

	body["auth"] = map[string]string{"type": "m.login.registration_token", "token": regToken, "session": flow.Session}

	var done struct {
		UserID      string `json:"user_id"`
		AccessToken string `json:"access_token"`
	}
	if err := a.call(http.MethodPost, "/_matrix/client/v3/register", body, &done); err != nil {
		return a, err
	}

	a.id, a.token = done.UserID, done.AccessToken

	return a, a.call(http.MethodPut, "/_matrix/client/v3/profile/"+url.PathEscape(a.id)+"/displayname",
		map[string]string{"displayname": display}, nil)
}

func (a account) createRoom(body map[string]any) (string, error) {
	var resp struct {
		RoomID string `json:"room_id"`
	}
	err := a.call(http.MethodPost, "/_matrix/client/v3/createRoom", body, &resp)

	return resp.RoomID, err
}

func (a account) join(room string) error {
	return a.call(http.MethodPost, "/_matrix/client/v3/join/"+url.PathEscape(room), map[string]any{}, nil)
}

var txn int

func (a account) send(room, eventType string, content map[string]any) (string, error) {
	txn++
	var resp struct {
		EventID string `json:"event_id"`
	}
	err := a.call(http.MethodPut, fmt.Sprintf("/_matrix/client/v3/rooms/%s/send/%s/seed%d",
		url.PathEscape(room), eventType, txn), content, &resp)

	return resp.EventID, err
}

func (a account) say(room, body string) (string, error) {
	return a.send(room, "m.room.message", map[string]any{"msgtype": "m.text", "body": body})
}

// Seeded is what the recording needs to know of the seeded server.
type Seeded struct {
	Admin, Typist account
	General       string
}

// Seed fills the scratch server with a small, believable community: the
// first account registered becomes its admin.
func Seed(base string) (Seeded, error) {
	people := []struct{ user, display, device string }{
		{"ada", "Ada Lovelace", "Element Desktop"},
		{"grace", "Grace Hopper", "Element X on Pixel 9"},
		{"alan", "Alan Turing", "nheko"},
		{"linus", "Linus Torvalds", "gomuks"},
		{"ken", "Ken Thompson", "Fractal"},
	}

	accounts := map[string]account{}
	for _, p := range people {
		a, err := register(base, p.user, p.display, p.device)
		if err != nil {
			return Seeded{}, fmt.Errorf("registering %s: %w", p.user, err)
		}
		accounts[p.user] = a
	}
	ada, grace, alan, linus, ken := accounts["ada"], accounts["grace"], accounts["alan"], accounts["linus"], accounts["ken"]

	general, err := ada.createRoom(map[string]any{
		"name": "General", "topic": "Everything phantom.chat — be kind, stay on topic-ish",
		"room_alias_name": "general", "preset": "public_chat", "visibility": "public",
	})
	if err != nil {
		return Seeded{}, err
	}
	announcements, err := ada.createRoom(map[string]any{
		"name": "Announcements", "topic": "Releases and maintenance windows",
		"room_alias_name": "announcements", "preset": "public_chat", "visibility": "public",
	})
	if err != nil {
		return Seeded{}, err
	}
	dev, err := ada.createRoom(map[string]any{
		"name": "Dev", "topic": "Hacking on phantom itself", "preset": "private_chat",
		"invite": []string{grace.id, linus.id},
	})
	if err != nil {
		return Seeded{}, err
	}
	dm, err := ada.createRoom(map[string]any{"preset": "trusted_private_chat", "is_direct": true, "invite": []string{grace.id}})
	if err != nil {
		return Seeded{}, err
	}

	for _, a := range []account{grace, alan, linus, ken} {
		if err := a.join("#general:" + serverName); err != nil {
			return Seeded{}, err
		}
		if err := a.join("#announcements:" + serverName); err != nil {
			return Seeded{}, err
		}
	}
	for _, a := range []account{grace, linus} {
		if err := a.join(dev); err != nil {
			return Seeded{}, err
		}
	}
	if err := grace.join(dm); err != nil {
		return Seeded{}, err
	}
	_ = ada.call(http.MethodPut, "/_matrix/client/v3/user/"+url.PathEscape(ada.id)+"/account_data/m.direct",
		map[string][]string{grace.id: {dm}}, nil)

	type line struct {
		who  account
		room string
		body string
	}
	script := []line{
		{ada, general, "morning all, the server moved to phantom last night"},
		{grace, general, "smooth so far, my sync came back in under a second"},
		{alan, general, "same here, and the new console is lovely"},
		{linus, general, "BUY CHEAP DOMAINS >>> spam.example <<<"},
		{grace, dev, "the admin API landed, every console section is live now"},
		{linus, dev, "nice, I'll try the federation view tonight"},
		{ada, dev, "reports go to the admin room as well"},
		{grace, dm, "can you make Alan a moderator in #general?"},
		{ada, dm, "will do after standup"},
	}

	var spam, welcome string
	for i, l := range script {
		id, err := l.who.say(l.room, l.body)
		if err != nil {
			return Seeded{}, err
		}
		switch i {
		case 0:
			welcome = id
		case 3:
			spam = id
		}
		time.Sleep(20 * time.Millisecond)
	}

	_, _ = ada.send(announcements, "m.room.message", map[string]any{
		"msgtype": "m.notice", "body": "phantom 0.1.0 is now running on phantom.chat",
	})

	_ = grace.call(http.MethodPost, "/_matrix/client/v3/rooms/"+url.PathEscape(general)+"/report/"+url.PathEscape(spam),
		map[string]any{"reason": "spam link in #general"}, nil)
	_ = ken.call(http.MethodPost, "/_matrix/client/v3/users/"+url.PathEscape(linus.id)+"/report",
		map[string]any{"reason": "posting spam in public rooms"}, nil)

	// General's conversation ends last, so the chat opens on it.
	for _, l := range []line{
		{ken, general, "uh, that last one looks like spam"},
		{ada, general, "on it, thanks Ken. reported and handled from the console"},
	} {
		if _, err := l.who.say(l.room, l.body); err != nil {
			return Seeded{}, err
		}
	}

	// An edit and a couple of reactions, so the timeline has them.
	_, _ = ada.send(general, "m.room.message", map[string]any{
		"msgtype": "m.text", "body": "* morning all, the server moved to phantom last night",
		"m.new_content": map[string]any{"msgtype": "m.text", "body": "morning all, the server moved to phantom last night, no downtime"},
		"m.relates_to":  map[string]any{"rel_type": "m.replace", "event_id": welcome},
	})
	for _, a := range []account{grace, alan, ken} {
		_, _ = a.send(general, "m.reaction", map[string]any{
			"m.relates_to": map[string]any{"rel_type": "m.annotation", "event_id": welcome, "key": "+1"},
		})
	}

	for _, up := range []struct {
		who         account
		name, ctype string
		size        int
	}{
		{ada, "launch-banner.png", "image/png", 184_000},
		{grace, "admin-api.pdf", "application/pdf", 1_240_000},
		{linus, "demo.mp4", "video/mp4", 2_600_000},
		{alan, "notes.txt", "text/plain", 9_000},
	} {
		if err := up.who.upload(up.name, up.ctype, up.size); err != nil {
			return Seeded{}, err
		}
	}

	admin := "/_phantom/admin/v1"
	_ = ada.call(http.MethodPost, admin+"/registration_tokens",
		map[string]any{"uses_allowed": 10, "expires_in_secs": 7 * 24 * 3600}, nil)
	_ = ada.call(http.MethodPost, admin+"/registration_tokens",
		map[string]any{"token": "fosdem-2026", "uses_allowed": 50}, nil)
	_ = ada.call(http.MethodPost, admin+"/backup", nil, nil)

	time.Sleep(time.Second)

	return Seeded{Admin: ada, Typist: grace, General: general}, nil
}

// Typing has the typist start typing in the room, for the chat recording.
func (s Seeded) Typing() {
	_ = s.Typist.call(http.MethodPut, "/_matrix/client/v3/rooms/"+url.PathEscape(s.General)+"/typing/"+url.PathEscape(s.Typist.id),
		map[string]any{"typing": true, "timeout": 30000}, nil)
}
