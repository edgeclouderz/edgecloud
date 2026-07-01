package middleware

import (
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
// A background goroutine can call GC periodically to evict buckets
// that haven't been touched in >10 minutes.
type RateLimiter struct {
	mu         sync.Mutex
	buckets    map[string]*bucket
	rate       float64 // tokens per second
	burst      float64 // max accumulated tokens
	lastGCTime time.Time
	gcInterval time.Duration
}

type bucket struct {
	tokens   float64
	lastTick time.Time
}

// NewRateLimiter creates a token-bucket rate limiter.
// rate: tokens added per second. burst: maximum accumulated tokens.
func NewRateLimiter(rate, burst int) *RateLimiter {
	return &RateLimiter{
		buckets:    make(map[string]*bucket),
		rate:       float64(rate),
		burst:      float64(burst),
		lastGCTime: time.Now(),
		gcInterval: 5 * time.Minute,
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
// Call periodically from a background goroutine.
func (rl *RateLimiter) GC() {
	rl.mu.Lock()
	defer rl.mu.Unlock()

	cutoff := time.Now().Add(-10 * time.Minute)
	for key, b := range rl.buckets {
		if b.lastTick.Before(cutoff) {
			delete(rl.buckets, key)
		}
	}
	rl.lastGCTime = time.Now()
}

// Middleware returns an HTTP middleware that rate-limits requests by
// a key extracted from the request via keyFunc. When the limit is
// exceeded, it responds with 429 Too Many Requests and a Retry-After
// header.
func (rl *RateLimiter) Middleware(keyFunc func(*http.Request) string) func(http.Handler) http.Handler {
	return func(next http.Handler) http.Handler {
		return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
			key := keyFunc(r)
			if key == "" {
				// No key means we can't rate-limit; pass through.
				next.ServeHTTP(w, r)
				return
			}

			// Run GC periodically inline (best-effort, every gcInterval).
			if time.Since(rl.lastGCTime) > rl.gcInterval {
				go rl.GC()
			}

			if !rl.Allow(key) {
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
func ClientIP(r *http.Request) string {
	if fwd := r.Header.Get("X-Forwarded-For"); fwd != "" {
		parts := strings.SplitN(fwd, ",", 2)
		return strings.TrimSpace(parts[0])
	}
	// Strip port from RemoteAddr (e.g. "192.168.1.1:54321").
	addr := r.RemoteAddr
	if idx := strings.LastIndex(addr, ":"); idx != -1 {
		return addr[:idx]
	}
	return addr
}
