//go:build ariarouter_ffi

package ariarouter

import (
	"encoding/json"
	"os"
	"testing"
)

func TestInitComplete(t *testing.T) {
	cfg := os.Getenv("ARIAROUTER_CONFIG")
	if cfg == "" {
		t.Skip("ARIAROUTER_CONFIG")
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
	out, err := r.Complete([]map[string]string{{"role": "user", "content": "hi"}}, map[string]string{"model": "aria/semantic-auto"})
	if err != nil {
		t.Fatal(err)
	}
	s, _ := json.Marshal(out)
	if !contains(string(s), "hello-from-router") {
		t.Fatalf("complete: %s", s)
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
