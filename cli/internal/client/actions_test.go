package client

import (
	"context"
	"encoding/json"
	"net/http"
	"testing"
)

func TestActionsUseTheirMethodsAndPaths(t *testing.T) {
	type call struct{ method, path, body string }
	var got []call

	mux := http.NewServeMux()
	mux.HandleFunc("/_phantom/admin/v1/", func(w http.ResponseWriter, r *http.Request) {
		var body map[string]any
		_ = json.NewDecoder(r.Body).Decode(&body)
		raw, _ := json.Marshal(body)
		got = append(got, call{r.Method, r.URL.EscapedPath(), string(raw)})

		_, _ = w.Write([]byte(`{"task_id":"T1","token":"abc","source":"database"}`))
	})
	c := serve(t, mux)
	ctx := context.Background()

	_ = c.Deactivate(ctx, "@bob:test", true)
	_ = c.SetPassword(ctx, "@bob:test", "pw", true)
	_ = c.RevokeAdmin(ctx, "@bob:test")
	_ = c.DeleteDevice(ctx, "@bob:test", "DEV")
	_ = c.BanRoom(ctx, "!r:test", false)
	id, _ := c.DeleteRoom(ctx, "!r:test", false)
	tok, _ := c.CreateToken(ctx, NewToken{})

	want := []call{
		{"POST", "/_phantom/admin/v1/users/@bob:test/deactivate", `{"erase":true}`},
		{"PUT", "/_phantom/admin/v1/users/@bob:test/password", `{"logout_devices":true,"password":"pw"}`},
		{"DELETE", "/_phantom/admin/v1/users/@bob:test/admin", `null`},
		{"DELETE", "/_phantom/admin/v1/devices/@bob:test/DEV", `null`},
		{"DELETE", "/_phantom/admin/v1/rooms/%21r:test/ban", `null`},
		{"DELETE", "/_phantom/admin/v1/rooms/%21r:test", `{"force":false}`},
		{"POST", "/_phantom/admin/v1/registration_tokens", `{}`},
	}
	if len(got) != len(want) {
		t.Fatalf("calls = %v", got)
	}
	for i := range want {
		if got[i] != want[i] {
			t.Errorf("call %d = %+v, want %+v", i, got[i], want[i])
		}
	}
	if id != "T1" || tok.Token != "abc" {
		t.Errorf("task %q token %q", id, tok.Token)
	}
}
