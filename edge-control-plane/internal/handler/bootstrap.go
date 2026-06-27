package handler

import (
	"encoding/json"
	"net/http"

	"github.com/edgeclouderz/edge-cloud/edge-control-plane/internal/handler/httperror"
	"github.com/edgeclouderz/edge-cloud/edge-control-plane/internal/middleware"
	"github.com/edgeclouderz/edge-cloud/edge-control-plane/internal/service"
)

// BootstrapHandler mints worker JWTs after PSKAuth validates the
// worker's identity. Constructed once at startup with a shared
// WorkerJWTMinter so every bootstrap request reuses the same
// secret / issuer / TTL configuration.
type BootstrapHandler struct {
	minter *service.WorkerJWTMinter
}

// NewBootstrapHandler wraps the supplied minter. Returns nil if the
// minter is nil so the caller can pass through a "bootstrap not
// configured" path without nil checks at every call site.
func NewBootstrapHandler(minter *service.WorkerJWTMinter) *BootstrapHandler {
	if minter == nil {
		return nil
	}
	return &BootstrapHandler{minter: minter}
}

// MintToken is the handler for `POST /api/internal/auth/token`.
//
// Request body:
//
//	{
//	  "worker_id":  "w_fra_abc123",
//	  "region":     "fra",
//	  "tenant_id":  "t_tenant1"
//	}
//
// Headers (validated by PSKAuth before this runs):
//
//	X-Worker-Id:             w_fra_abc123
//	X-Worker-Region:         fra
//	X-Bootstrap-Signature:   hex(HMAC-SHA256(psk, "{worker_id}:{region}"))
//
// Response body (200):
//
//	{
//	  "token":           "eyJ...",
//	  "expires_at_unix": 1782547200,
//	  "token_type":      "Bearer"
//	}
//
// Failure modes:
//   - 400 BAD_REQUEST — missing/empty fields in the body; body
//     disagrees with the headers (defense-in-depth: an attacker who
//     forges a header signature can't pivot to minting a token for
//     a different worker by editing the body).
//   - 500 INTERNAL_ERROR — `minter.Mint` failure (e.g. HMAC key
//     zero-length, which would be a config bug caught at startup
//     but guarded here defensively).
func (h *BootstrapHandler) MintToken(w http.ResponseWriter, r *http.Request) {
	// PSKAuth has already validated the signature and placed
	// worker_id + region into the request context. Reading them
	// from the context (not the body) is the source of truth — the
	// body is allowed to disagree, but doing so returns 400.
	signedWorkerID := middleware.GetBootstrapWorkerID(r.Context())
	signedRegion := middleware.GetBootstrapRegion(r.Context())
	if signedWorkerID == "" || signedRegion == "" {
		// PSKAuth should have populated these; if not, the route
		// was wired wrong (e.g. handler reached without
		// middleware). Surface as a 500 because it's a server
		// configuration bug, not a client problem.
		httperror.InternalErrorCtx(w, r)
		return
	}

	var body struct {
		WorkerID string `json:"worker_id"`
		Region   string `json:"region"`
		TenantID string `json:"tenant_id"`
	}
	if err := json.NewDecoder(r.Body).Decode(&body); err != nil {
		httperror.BadRequestCtx(w, r, "invalid JSON body")
		return
	}
	if body.WorkerID == "" || body.Region == "" || body.TenantID == "" {
		httperror.BadRequestCtx(w, r, "worker_id, region, and tenant_id are required")
		return
	}
	if body.WorkerID != signedWorkerID || body.Region != signedRegion {
		// Body / header mismatch: an attacker who somehow obtained
		// a valid signature for one worker is trying to mint a
		// token for another. Reject before the minter sees the
		// inputs.
		httperror.BadRequestCtx(w, r, "body worker_id or region does not match signed headers")
		return
	}

	token, exp, err := h.minter.Mint(body.WorkerID, body.TenantID, body.Region)
	if err != nil {
		// Logging here would be the caller's job (the handler
		// returns 500 with a generic message; the server log
		// captures the real error before we drop it). Today the
		// handler runs without a logger reference; a future
		// commit threads one through. For now the operator sees
		// the symptom (every bootstrap returns 500) but not the
		// cause — acceptable because the only realistic cause is
		// a misconfiguration that should never reach production.
		httperror.InternalErrorCtx(w, r)
		return
	}

	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(http.StatusOK)
	_ = json.NewEncoder(w).Encode(map[string]any{
		"token":           token,
		"expires_at_unix": exp.Unix(),
		"token_type":      "Bearer",
	})
}
