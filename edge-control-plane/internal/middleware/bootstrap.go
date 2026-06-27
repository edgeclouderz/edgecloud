package middleware

import (
	"context"
	"crypto/hmac"
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"net/http"
	"strings"

	"github.com/edgeclouderz/edge-cloud/edge-control-plane/internal/handler/httperror"
)

// BootstrapAuthConfig holds the pre-shared key the worker uses to
// prove its identity during enrollment. Separate from WorkerJWTConfig
// because the bootstrap path is the chicken-and-egg predecessor of
// the JWT path — same server, different proof mechanism.
type BootstrapAuthConfig struct {
	// PSK is the pre-shared key the worker HMAC-signs with. Must match
	// WORKER_BOOTSTRAP_PSK on the worker side. Empty PSK disables
	// the endpoint entirely (every request is rejected with 503 —
	// the route exists but cannot succeed until configured).
	PSK []byte
}

const (
	// BootstrapWorkerIDKey / BootstrapRegionKey carry the validated
	// identity from PSKAuth into the handler via context. Named
	// distinctly from WorkerIDKey / WorkerRegionKey so a handler that
	// wants WorkerAuth claims (post-bootstrap) doesn't accidentally
	// read a bootstrap-time identity that hasn't been promoted to a
	// full JWT yet.
	BootstrapWorkerIDKey contextKey = "bootstrap_worker_id"
	BootstrapRegionKey   contextKey = "bootstrap_region"
)

// VerifyPSKSignature checks that hex(HMAC-SHA256(psk, "{worker_id}:{region}"))
// matches the supplied `signatureHex`. Returns nil on success and an
// error describing the failure otherwise.
//
// `signatureHex` must be a 64-char lowercase hex digest (the same
// shape the worker produces via `bootstrap::sign_with_psk`). Mismatched
// length, non-hex chars, or wrong digest all return errors without
// distinguishing which condition failed — an attacker probing the
// endpoint shouldn't learn whether their guess had a valid format.
//
// `hmac.Equal` (constant-time) is used for the final byte comparison
// so an attacker can't time-side-channel the signature. The early
// returns on length / hex errors are not constant-time, but they leak
// at most "your input shape was wrong" which is already public
// information — the worker always sends a 64-char hex string.
func VerifyPSKSignature(psk []byte, workerID, region, signatureHex string) error {
	if len(signatureHex) != 64 {
		return errors.New("signature must be 64-char hex")
	}
	if _, err := hex.DecodeString(signatureHex); err != nil {
		return errors.New("signature must be valid hex")
	}
	mac := hmac.New(sha256.New, psk)
	mac.Write([]byte(workerID))
	mac.Write([]byte(":"))
	mac.Write([]byte(region))
	expected := mac.Sum(nil)
	got, err := hex.DecodeString(signatureHex)
	if err != nil {
		// Already checked above, but be defensive — hex.DecodeString
		// on a 64-char lowercase hex string can't fail here.
		return errors.New("signature decode failed")
	}
	if !hmac.Equal(expected, got) {
		return errors.New("signature mismatch")
	}
	return nil
}

// PSKAuth returns a middleware that validates the X-Bootstrap-Signature
// header against the configured PSK. On success, worker_id and region
// from the X-Worker-Id / X-Worker-Region headers are placed into the
// request context for the handler to read.
//
// **Route precedence:** this middleware is intended for the OUTER mux
// (not inside the WorkerAuth subtree). Go 1.22+'s ServeMux matches
// the most specific pattern first, so registering
// `POST /api/internal/auth/token` on the outer mux wins over
// `/api/internal/` despite living on the same mux. See cmd/api/main.go
// for the exact wiring.
//
// On failure, returns 401 (invalid signature) or 400 (missing /
// malformed headers). The 401 message is deliberately generic
// ("invalid signature") to avoid leaking whether the worker_id or
// region matched and only the PSK didn't — an attacker shouldn't be
// able to enumerate valid worker identities by watching for
// differentiated error messages.
func PSKAuth(cfg BootstrapAuthConfig) func(http.Handler) http.Handler {
	return func(next http.Handler) http.Handler {
		return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
			if len(cfg.PSK) == 0 {
				// 503 (not 401) because the server itself is
				// misconfigured — not the client's fault. Operators
				// see this in their own logs and know to set
				// BOOTSTRAP_PSK.
				httperror.WriteCtx(w, r, http.StatusServiceUnavailable, "bootstrap disabled: BOOTSTRAP_PSK not configured")
				return
			}
			workerID := strings.TrimSpace(r.Header.Get("X-Worker-Id"))
			region := strings.TrimSpace(r.Header.Get("X-Worker-Region"))
			signature := r.Header.Get("X-Bootstrap-Signature")
			if workerID == "" || region == "" || signature == "" {
				httperror.BadRequestCtx(w, r, "missing X-Worker-Id, X-Worker-Region, or X-Bootstrap-Signature header")
				return
			}
			if err := VerifyPSKSignature(cfg.PSK, workerID, region, signature); err != nil {
				httperror.UnauthorizedCtx(w, r, "invalid signature")
				return
			}
			ctx := context.WithValue(r.Context(), BootstrapWorkerIDKey, workerID)
			ctx = context.WithValue(ctx, BootstrapRegionKey, region)
			next.ServeHTTP(w, r.WithContext(ctx))
		})
	}
}

// GetBootstrapWorkerID extracts the bootstrap-time worker ID from
// context. Returns "" if the request wasn't PSKAuth-authenticated.
func GetBootstrapWorkerID(ctx context.Context) string {
	if id, ok := ctx.Value(BootstrapWorkerIDKey).(string); ok {
		return id
	}
	return ""
}

// GetBootstrapRegion extracts the bootstrap-time region from context.
func GetBootstrapRegion(ctx context.Context) string {
	if r, ok := ctx.Value(BootstrapRegionKey).(string); ok {
		return r
	}
	return ""
}
