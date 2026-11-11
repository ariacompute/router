//go:build aria_router_ffi

package aria

import (
	"encoding/json"
	"os"
	"testing"
)

func TestInitComplete(t *testing.T) {
	cfg := os.Getenv("ARIA_ROUTER_CONFIG")
	if cfg == "" {
		t.Skip("ARIA_ROUTER_CONFIG")
	}
	r := NewRouter()
	if err := r.Init(cfg); err != nil {
		t.Fatal(err)
	}
	defer r.Close()
	m, err := r.Models()
	if err != nil {
		t.Fatal(err)
	}
	raw, _ := json.Marshal(m["data"])
	if !contains(string(raw), "semantic-auto") {
		t.Fatalf("models: %s", raw)
	}
	out, err := r.Complete([]map[string]string{{"role": "user", "content": "hi"}}, map[string]string{"model": "ariacompute/semantic-auto"})
	if err != nil {
		t.Fatal(err)
	}
	s, _ := json.Marshal(out)
	if !contains(string(s), "hello-from-router") {
		t.Fatalf("complete: %s", s)
	}
}

func TestConnect(t *testing.T) {
	if os.Getenv("ARIA_ROUTER_FFI_LIB") == "" {
		t.Skip("ARIA_ROUTER_FFI_LIB")
	}
	r := NewRouter()
	if err := r.Connect("http://127.0.0.1:9"); err != nil {
		t.Fatal(err)
	}
	r.Close()
}

func TestSetupStatusAndStream(t *testing.T) {
	cfg := os.Getenv("ARIA_ROUTER_CONFIG")
	if cfg == "" {
		t.Skip("ARIA_ROUTER_CONFIG")
	}
	r := NewRouter()
	r.Setup("http://127.0.0.1:8899", "t")
	if got := r.SetupStatus(); got.Token != "t" || got.BaseURL != "http://127.0.0.1:8899" {
		t.Fatalf("setup status: %+v", got)
	}
	r.SetupClear()
	if got := r.SetupStatus(); got.Token != "" || got.BaseURL != "" {
		t.Fatalf("setup clear: %+v", got)
	}
	if err := r.Init(cfg); err != nil {
		t.Fatal(err)
	}
	defer r.Destroy()
	chunks := 0
	out, err := r.CompleteStream(
		[]map[string]string{{"role": "user", "content": "hi"}},
		map[string]string{"model": "ariacompute/semantic-auto"},
		func(s string) {
			chunks++
			if !contains(s, "hello-from-router") {
				t.Errorf("stream chunk missing canned reply: %s", s)
			}
		},
	)
	if err != nil {
		t.Fatal(err)
	}
	if chunks != 1 {
		t.Fatalf("expected 1 chunk, got %d", chunks)
	}
	s, _ := json.Marshal(out)
	if !contains(string(s), "hello-from-router") {
		t.Fatalf("complete_stream: %s", s)
	}
	lr, err := r.LastRoute()
	if err != nil {
		t.Fatal(err)
	}
	if _, ok := lr["headers"]; !ok {
		t.Fatalf("last_route missing headers: %v", lr)
	}
}

func contains(s, sub string) bool {
	return len(s) >= len(sub) && (s == sub || len(sub) == 0 || (func() bool {
		for i := 0; i+len(sub) <= len(s); i++ {
			if s[i:i+len(sub)] == sub {
				return true
			}
		}
		return false
	})())
}
