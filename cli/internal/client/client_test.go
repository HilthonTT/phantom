package client

import (
	"context"
	"errors"
	"net/http"
	"net/http/httptest"
	"testing"
)

// phantom answers the probed routes the way phantom-server does, with
// federation switched by the flag.
func phantom(t *testing.T, federation bool) *Client {
	t.Helper()

	mux := http.NewServeMux()
	mux.HandleFunc("GET /_phantom/server_version", func(w http.ResponseWriter, _ *http.Request) {
		_, _ = w.Write([]byte(`{"name":"phantom","version":"0.1.0"}`))
	})
	mux.HandleFunc("GET /_matrix/client/versions", func(w http.ResponseWriter, _ *http.Request) {
		_, _ = w.Write([]byte(`{"versions":["v1.1","v1.18"],"unstable_features":{"a":true,"b":true,"c":false}}`))
	})
	if federation {
		mux.HandleFunc("GET /_matrix/key/v2/server", func(w http.ResponseWriter, _ *http.Request) {
			_, _ = w.Write([]byte(`{"server_name":"phantom.test"}`))
		})
		mux.HandleFunc("GET /_phantom/local_user_count", func(w http.ResponseWriter, _ *http.Request) {
			_, _ = w.Write([]byte(`{"count":7}`))
		})
	}
	mux.HandleFunc("/", func(w http.ResponseWriter, _ *http.Request) {
		w.WriteHeader(http.StatusNotFound)
		_, _ = w.Write([]byte(`{"errcode":"M_UNRECOGNIZED","error":"Unrecognized request"}`))
	})

	srv := httptest.NewServer(mux)
	t.Cleanup(srv.Close)

	c, err := New(srv.URL)
	if err != nil {
		t.Fatal(err)
	}

	return c
}

func TestProbeReadsEveryRoute(t *testing.T) {
	s, err := phantom(t, true).Probe(context.Background())
	if err != nil {
		t.Fatal(err)
	}

	want := Status{
		Software: "phantom", Version: "0.1.0",
		ServerName: "phantom.test", Federation: true,
		Spec: "v1.18", Unstable: 2,
		LocalUsers: 7,
	}
	s.Latency = 0
	if s != want {
		t.Errorf("Probe = %+v, want %+v", s, want)
	}
}

func TestProbeWithFederationOff(t *testing.T) {
	s, err := phantom(t, false).Probe(context.Background())
	if err != nil {
		t.Fatal(err)
	}

	if s.Federation || s.ServerName != "" || s.LocalUsers != -1 {
		t.Errorf("Probe = %+v, want federation off with name and count withheld", s)
	}
}

func TestProbeFailsWithoutTheVersionRoute(t *testing.T) {
	srv := httptest.NewServer(http.NotFoundHandler())
	t.Cleanup(srv.Close)

	c, err := New(srv.URL)
	if err != nil {
		t.Fatal(err)
	}

	_, err = c.Probe(context.Background())
	if se, ok := errors.AsType[*StatusError](err); !ok || se.Code != http.StatusNotFound {
		t.Fatalf("Probe err = %v, want a 404 StatusError", err)
	}
}

func TestNewTakesABareHostAsHTTP(t *testing.T) {
	c, err := New("localhost:8008/")
	if err != nil {
		t.Fatal(err)
	}

	if got := c.URL(); got != "http://localhost:8008" {
		t.Errorf("URL = %q, want http://localhost:8008", got)
	}
}

func TestNewRejectsOtherSchemes(t *testing.T) {
	if _, err := New("ftp://localhost"); err == nil {
		t.Error("New accepted an ftp URL")
	}
}
