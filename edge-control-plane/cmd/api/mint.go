package main

import (
	"fmt"
	"time"

	"github.com/edgeclouderz/edge-cloud/edge-control-plane/internal/middleware"
	"github.com/golang-jwt/jwt/v5"
)

// mintIngressToken builds a long-lived (1y TTL) HMAC-SHA256 JWT for the
// ingress binary. The token carries `role: "ingest"` so the
// `WorkerAuth` middleware can distinguish it from per-worker tokens
// (which carry `role: "worker"`, or no `role` for backward compat).
//
// The token is NOT a per-process secret. It's the same on every restart
// unless the operator rotates `JWT_SECRET`. Operators SHOULD rotate
// JWT_SECRET periodically; rotating invalidates both the new token
// printed here AND every existing worker JWT, so it requires a
// coordinated control-plane + worker + ingress redeploy. This is
// intentional — the ingress and workers are managed together.
func mintIngressToken(secret, issuer, region string) (string, error) {
	if secret == "" {
		return "", fmt.Errorf("JWT secret is empty")
	}
	if region == "" {
		region = "global"
	}
	now := time.Now()
	claims := middleware.WorkerClaims{
		RegisteredClaims: jwt.RegisteredClaims{
			Issuer:    issuer,
			Subject:   "ingress-" + region,
			IssuedAt:  jwt.NewNumericDate(now),
			ExpiresAt: jwt.NewNumericDate(now.Add(365 * 24 * time.Hour)),
			NotBefore: jwt.NewNumericDate(now.Add(-1 * time.Minute)),
		},
		WorkerID: "ingress-" + region,
		// TenantID is intentionally empty — the ingress is a global
		// service, not bound to a single tenant. Internal endpoints
		// (`ListDomains`, `TlsAllowed`) are tenant-agnostic.
		Role: middleware.RoleIngest,
	}
	token := jwt.NewWithClaims(jwt.SigningMethodHS256, claims)
	signed, err := token.SignedString([]byte(secret))
	if err != nil {
		return "", fmt.Errorf("signing ingress token: %w", err)
	}
	return signed, nil
}
