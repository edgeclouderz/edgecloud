package middleware

import (
	"net"
	"net/http"
	"strings"
	"sync"
	"time"

	"github.com/edgeclouderz/edge-cloud/edge-control-plane/internal/handler/httperror"
)

// RateLimiter is a token-bucket rate limiter keyed by a string
// (tenant ID or client IP). It is safe for concurrent use.
//
// Each key has a bucket that fills at `rate` tokens per second
// up to a maximum of `burst`. A call to Allow consumes one token.
// When the bucket is empty, the call is denied.
//
// A background goroutine evicts stale (>10min untouched) buckets
// every 5 minutes.
type RateLimiter struct {
	mu       sync.Mutex
	buckets  map[string]*bucket
	rate     float64 // tokens per second
	burst    float64 // max accumulated tokens
	disabled bool    // when true, Middleware is a no-op
	stopCh   chan struct{}
}

type bucket struct {
	tokens   float64
	lastTick time.Time
}

// NewRateLimiter creates a token-bucket rate limiter.
// rate: tokens added per second. burst: maximum accumulated tokens.
// If rate <= 0 or burst <= 0, the limiter is disabled (no-op).
func NewRateLimiter(rate, burst int) *RateLimiter {
	rl := &RateLimiter{
		buckets:  make(map[string]*bucket),
		rate:     float64(rate),
		burst:    float64(burst),
		disabled: rate <= 0 || burst <= 0,
		stopCh:   make(chan struct{}),
	}
	if !rl.disabled {
		go rl.gcLoop()
	}
	return rl
}

// gcLoop runs GC every 5 minutes until the limiter is stopped.
func (rl *RateLimiter) gcLoop() {
	ticker := time.NewTicker(5 * time.Minute)
	defer ticker.Stop()
	for {
		select {
		case <-ticker.C:
			rl.GC()
		case <-rl.stopCh:
			return
		}
	}
}

// Stop shuts down the background GC goroutine. Safe to call multiple times.
// After Stop, the limiter still works but stale buckets are never cleaned up.
func (rl *RateLimiter) Stop() {
	if !rl.disabled {
		select {
		case <-rl.stopCh:
		default:
			close(rl.stopCh)
		}
	}
}

// Allow reports whether one token can be consumed for the given key.
func (rl *RateLimiter) Allow(key string) bool {
	rl.mu.Lock()
	defer rl.mu.Unlock()

	b, ok := rl.buckets[key]
	now := time.Now()

	if !ok {
		// New bucket: start with burst-1 tokens (one consumed now).
		b = &bucket{tokens: rl.burst - 1, lastTick: now}
		rl.buckets[key] = b
		return true
	}

	// Refill tokens based on elapsed time.
	elapsed := now.Sub(b.lastTick).Seconds()
	b.tokens += elapsed * rl.rate
	if b.tokens > rl.burst {
		b.tokens = rl.burst
	}
	b.lastTick = now

	if b.tokens >= 1 {
		b.tokens--
		return true
	}
	return false
}

// GC removes buckets that haven't been touched in >10 minutes.
func (rl *RateLimiter) GC() {
	rl.mu.Lock()
	defer rl.mu.Unlock()

	cutoff := time.Now().Add(-10 * time.Minute)
	for key, b := range rl.buckets {
		if b.lastTick.Before(cutoff) {
			delete(rl.buckets, key)
		}
	}
}

// Middleware returns an HTTP middleware that rate-limits requests by
// a key extracted from the request via keyFunc. When disabled (rate <= 0
// or burst <= 0), the middleware is a no-op. When the limit is exceeded,
// it responds with 429 Too Many Requests and a Retry-After header.
func (rl *RateLimiter) Middleware(keyFunc func(*http.Request) string) func(http.Handler) http.Handler {
	if rl.disabled {
		return func(next http.Handler) http.Handler {
			return next
		}
	}

	return func(next http.Handler) http.Handler {
		return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
			key := keyFunc(r)
			if key == "" {
				next.ServeHTTP(w, r)
				return
			}

			if !rl.Allow(key) {
				// Retry-After: 1 second is a safe lower bound.
				// At the configured rate, one token replenishes in (1/rate) seconds.
				w.Header().Set("Retry-After", "1")
				httperror.QuotaExceededCtx(w, r, "rate limit exceeded")
				return
			}

			next.ServeHTTP(w, r)
		})
	}
}

// ClientIP extracts the client IP address from a request, checking
// the X-Forwarded-For header first and falling back to RemoteAddr.
// Handles both IPv4 (with port) and IPv6 (bracketed with port).
func ClientIP(r *http.Request) string {
	if fwd := r.Header.Get("X-Forwarded-For"); fwd != "" {
		parts := strings.SplitN(fwd, ",", 2)
		return strings.TrimSpace(parts[0])
	}

	// net.SplitHostPort handles both "192.168.1.1:54321" and "[::1]:8080".
	host, _, err := net.SplitHostPort(r.RemoteAddr)
	if err != nil {
		// No port — likely direct IPv4/IPv6 without port.
		return r.RemoteAddr
	}
	return host
}
