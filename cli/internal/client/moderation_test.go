package client

import (
	"context"
	"encoding/json"
	"net/http"
	"testing"
)

func TestModerationActionsUseTheirMethodsAndPaths(t *testing.T) {
	type call struct{ method, path, body string }
	var got []call

	mux := http.NewServeMux()
	mux.HandleFunc("/_phantom/admin/v1/", func(w http.ResponseWriter, r *http.Request) {
		var body map[string]any
		_ = json.NewDecoder(r.Body).Decode(&body)
		raw, _ := json.Marshal(body)
		got = append(got, call{r.Method, r.URL.EscapedPath(), string(raw)})

		_, _ = w.Write([]byte(`{"task_id":"T1","still_blocked_by":["evil"]}`))
	})
	c := serve(t, mux)
	ctx := context.Background()

	_ = c.BlockServer(ctx, "evil.example")
	still, _ := c.UnblockServer(ctx, "evil.example")
	_ = c.QuarantineMedia(ctx, "mxc://test/abc", true)
	_ = c.QuarantineMedia(ctx, "mxc://test/abc", false)
	id, _ := c.PurgeHistory(ctx, "!r:test", 1000)
	_, _ = c.PurgeOldMedia(ctx, "remote", 30)

	want := []call{
		{"POST", "/_phantom/admin/v1/federation/blocks", `{"server":"evil.example"}`},
		{"DELETE", "/_phantom/admin/v1/federation/blocks", `{"server":"evil.example"}`},
		{"PUT", "/_phantom/admin/v1/media/test/abc/quarantine", `null`},
		{"DELETE", "/_phantom/admin/v1/media/test/abc/quarantine", `null`},
		{"POST", "/_phantom/admin/v1/rooms/%21r:test/purge_history", `{"before_ts_ms":1000}`},
		{"POST", "/_phantom/admin/v1/media/purge", `{"older_than_days":30,"origin":"remote"}`},
	}
	if len(got) != len(want) {
		t.Fatalf("calls = %v", got)
	}
	for i := range want {
		if got[i] != want[i] {
			t.Errorf("call %d = %+v, want %+v", i, got[i], want[i])
		}
	}
	if id != "T1" || len(still) != 1 || still[0] != "evil" {
		t.Errorf("task %q still blocked by %v", id, still)
	}

	if err := c.QuarantineMedia(ctx, "mxc://test/", true); err == nil {
		t.Error("an mxc URI without a media ID was accepted")
	}
}
