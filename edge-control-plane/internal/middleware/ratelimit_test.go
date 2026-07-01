package middleware

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"sync"
	"testing"
	"time"
)

// TestRateLimiter_AllowsUpToBurst confirms that burst requests are
// allowed, and the (burst+1)th is denied.
func TestRateLimiter_AllowsUpToBurst(t *testing.T) {
	rl := NewRateLimiter(10, 5)
	defer rl.Stop()

	for i := 0; i < 5; i++ {
		if !rl.Allow("key-a") {
			t.Fatalf("request %d: expected allowed, got denied", i+1)
		}
	}
	if rl.Allow("key-a") {
		t.Fatal("expected 6th request to be denied")
	}
}

// TestRateLimiter_RefillsOverTime confirms that after waiting,
// tokens are replenished and a subsequent request succeeds.
func TestRateLimiter_RefillsOverTime(t *testing.T) {
	rl := NewRateLimiter(10, 1)
	defer rl.Stop()

	if !rl.Allow("key-b") {
		t.Fatal("expected first request to be allowed")
	}
	if rl.Allow("key-b") {
		t.Fatal("expected second request to be denied (burst exhausted)")
	}

	time.Sleep(150 * time.Millisecond)

	if !rl.Allow("key-b") {
		t.Fatal("expected request after refill to be allowed")
	}
}

// TestRateLimiter_MultipleKeysIndependent confirms that one key
// exhausting its burst doesn't affect another key.
func TestRateLimiter_MultipleKeysIndependent(t *testing.T) {
	rl := NewRateLimiter(10, 3)
	defer rl.Stop()

	for i := 0; i < 3; i++ {
		rl.Allow("key-a")
	}

	if !rl.Allow("key-b") {
		t.Fatal("expected key-b to be allowed independently")
	}
}

// TestRateLimiter_ConcurrentSafe confirms that Allow can be called
// from multiple goroutines without panicking.
func TestRateLimiter_ConcurrentSafe(t *testing.T) {
	rl := NewRateLimiter(100, 50)
	defer rl.Stop()

	var wg sync.WaitGroup
	for i := 0; i < 10; i++ {
		wg.Add(1)
		go func(id int) {
			defer wg.Done()
			for j := 0; j < 20; j++ {
				key := "key-" + string(rune('a'+id))
				rl.Allow(key)
			}
		}(i)
	}
	wg.Wait()
}

// TestRateLimiter_GC removes stale entries and leaves active ones.
func TestRateLimiter_GC(t *testing.T) {
	rl := NewRateLimiter(10, 5)
	defer rl.Stop()

	rl.Allow("stale-key")
	rl.Allow("fresh-key")

	rl.mu.Lock()
	if b, ok := rl.buckets["stale-key"]; ok {
		b.lastTick = time.Now().Add(-15 * time.Minute)
	}
	rl.mu.Unlock()

	rl.GC()

	rl.mu.Lock()
	_, hasStale := rl.buckets["stale-key"]
	_, hasFresh := rl.buckets["fresh-key"]
	rl.mu.Unlock()

	if hasStale {
		t.Error("expected stale-key to be evicted by GC")
	}
	if !hasFresh {
		t.Error("expected fresh-key to survive GC")
	}
}

// TestRateLimiter_Disabled confirms that rate <= 0 or burst <= 0
// creates a disabled limiter whose Middleware is a no-op.
func TestRateLimiter_Disabled(t *testing.T) {
	rl := NewRateLimiter(0, 0) // disabled

	handler := rl.Middleware(func(r *http.Request) string {
		return "any-key"
	})(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusOK)
	}))

	for i := 0; i < 5; i++ {
		req := httptest.NewRequest(http.MethodGet, "/", nil)
		w := httptest.NewRecorder()
		handler.ServeHTTP(w, req)
		if w.Code != http.StatusOK {
			t.Fatalf("request %d: expected 200 with disabled limiter, got %d", i+1, w.Code)
		}
	}
}

// TestRateLimiter_Middleware_Returns429 confirms the middleware
// returns 429 with Retry-After when the limit is exceeded.
func TestRateLimiter_Middleware_Returns429(t *testing.T) {
	rl := NewRateLimiter(10, 2)
	defer rl.Stop()

	handler := rl.Middleware(func(r *http.Request) string {
		return "test-tenant"
	})(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusOK)
	}))

	for i := 0; i < 2; i++ {
		req := httptest.NewRequest(http.MethodGet, "/", nil)
		w := httptest.NewRecorder()
		handler.ServeHTTP(w, req)
		if w.Code != http.StatusOK {
			t.Fatalf("request %d: expected 200, got %d", i+1, w.Code)
		}
	}

	req := httptest.NewRequest(http.MethodGet, "/", nil)
	w := httptest.NewRecorder()
	handler.ServeHTTP(w, req)
	if w.Code != http.StatusTooManyRequests {
		t.Fatalf("expected 429, got %d", w.Code)
	}

	if retryAfter := w.Header().Get("Retry-After"); retryAfter != "1" {
		t.Errorf("expected Retry-After: 1, got %q", retryAfter)
	}

	var body map[string]interface{}
	if err := json.NewDecoder(w.Body).Decode(&body); err != nil {
		t.Fatalf("decode error envelope: %v", err)
	}
	errObj, ok := body["error"].(map[string]interface{})
	if !ok {
		t.Fatal("missing 'error' key in response")
	}
	if errObj["code"] != "QUOTA_EXCEEDED" {
		t.Errorf("expected code QUOTA_EXCEEDED, got %v", errObj["code"])
	}
}

// TestRateLimiter_Middleware_EmptyKeyPassthrough confirms that when
// keyFunc returns "", the request passes through without rate limiting.
func TestRateLimiter_Middleware_EmptyKeyPassthrough(t *testing.T) {
	rl := NewRateLimiter(10, 2)
	defer rl.Stop()

	handler := rl.Middleware(func(r *http.Request) string {
		return "" // no key — skip rate limiting
	})(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusOK)
	}))

	for i := 0; i < 5; i++ {
		req := httptest.NewRequest(http.MethodGet, "/", nil)
		w := httptest.NewRecorder()
		handler.ServeHTTP(w, req)
		if w.Code != http.StatusOK {
			t.Fatalf("request %d: expected 200 with empty key, got %d", i+1, w.Code)
		}
	}
}

// TestClientIP_FromXForwardedFor checks X-Forwarded-For extraction.
func TestClientIP_FromXForwardedFor(t *testing.T) {
	r := httptest.NewRequest(http.MethodGet, "/", nil)
	r.Header.Set("X-Forwarded-For", "203.0.113.1, 10.0.0.1")
	if ip := ClientIP(r); ip != "203.0.113.1" {
		t.Errorf("expected 203.0.113.1, got %q", ip)
	}
}

// TestClientIP_FromRemoteAddrIPv4 checks RemoteAddr with IPv4 + port.
func TestClientIP_FromRemoteAddrIPv4(t *testing.T) {
	r := httptest.NewRequest(http.MethodGet, "/", nil)
	r.RemoteAddr = "198.51.100.42:8080"
	if ip := ClientIP(r); ip != "198.51.100.42" {
		t.Errorf("expected 198.51.100.42, got %q", ip)
	}
}

// TestClientIP_FromRemoteAddrIPv6 checks RemoteAddr with IPv6 + port.
func TestClientIP_FromRemoteAddrIPv6(t *testing.T) {
	r := httptest.NewRequest(http.MethodGet, "/", nil)
	r.RemoteAddr = "[::1]:8080"
	if ip := ClientIP(r); ip != "::1" {
		t.Errorf("expected ::1, got %q", ip)
	}
}

// TestClientIP_FromRemoteAddrIPv6NoPort checks bare IPv6 without port.
func TestClientIP_FromRemoteAddrIPv6NoPort(t *testing.T) {
	r := httptest.NewRequest(http.MethodGet, "/", nil)
	r.RemoteAddr = "::1"
	if ip := ClientIP(r); ip != "::1" {
		t.Errorf("expected ::1, got %q", ip)
	}
}

// TestClientIP_NoPort checks RemoteAddr without port.
func TestClientIP_NoPort(t *testing.T) {
	r := httptest.NewRequest(http.MethodGet, "/", nil)
	r.RemoteAddr = "10.0.0.1"
	if ip := ClientIP(r); ip != "10.0.0.1" {
		t.Errorf("expected 10.0.0.1, got %q", ip)
	}
}

// TestRateLimiter_StopIdempotent confirms Stop can be called multiple times.
func TestRateLimiter_StopIdempotent(t *testing.T) {
	rl := NewRateLimiter(10, 5)
	rl.Stop()
	rl.Stop() // should not panic
}
