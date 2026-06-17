package middleware

import (
	"context"
	"errors"
	"net/http"
	"strings"

	"github.com/edgeclouderz/edge-cloud/edge-control-plane/internal/service"
)

// AuthMiddleware validates API keys and injects tenant context.
//
// It delegates the actual verification to APIKeyService.AuthenticateRawKey,
// which supports both argon2id (new keys) and legacy SHA-256 with lazy
// rehash — see issue #36.
type AuthMiddleware struct {
	apiKeySvc *service.APIKeyService
}

func NewAuthMiddleware(apiKeySvc *service.APIKeyService) *AuthMiddleware {
	return &AuthMiddleware{apiKeySvc: apiKeySvc}
}

// contextKey is a custom type for context keys to avoid collisions.
type contextKey string

const TenantIDKey contextKey = "tenant_id"
const APIKeyIDKey contextKey = "api_key_id"
const RoleKey contextKey = "role"

// Authenticate extracts and validates the Bearer token.
func (m *AuthMiddleware) Authenticate(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		authHeader := r.Header.Get("Authorization")
		if authHeader == "" {
			http.Error(w, `{"error": "missing authorization header"}`, http.StatusUnauthorized)
			return
		}

		parts := strings.SplitN(authHeader, " ", 2)
		if len(parts) != 2 || strings.ToLower(parts[0]) != "bearer" {
			http.Error(w, `{"error": "invalid authorization format"}`, http.StatusUnauthorized)
			return
		}

		apiKey, err := m.apiKeySvc.AuthenticateRawKey(r.Context(), parts[1])
		if err != nil {
			if errors.Is(err, service.ErrInvalidAPIKey) {
				http.Error(w, `{"error": "invalid api key"}`, http.StatusUnauthorized)
				return
			}
			http.Error(w, `{"error": "internal error"}`, http.StatusInternalServerError)
			return
		}

		// Inject into context
		ctx := context.WithValue(r.Context(), TenantIDKey, apiKey.TenantID)
		ctx = context.WithValue(ctx, APIKeyIDKey, apiKey.ID)
		ctx = context.WithValue(ctx, RoleKey, apiKey.Role)

		next.ServeHTTP(w, r.WithContext(ctx))
	})
}

// RequireRole checks that the authenticated user has one of the allowed roles.
func RequireRole(allowed ...string) func(http.Handler) http.Handler {
	return func(next http.Handler) http.Handler {
		return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
			role, ok := r.Context().Value(RoleKey).(string)
			if !ok {
				http.Error(w, `{"error": "unauthorized"}`, http.StatusUnauthorized)
				return
			}

			for _, a := range allowed {
				if role == a {
					next.ServeHTTP(w, r)
					return
				}
			}

			http.Error(w, `{"error": "forbidden"}`, http.StatusForbidden)
		})
	}
}

// GetTenantID extracts tenant ID from context.
func GetTenantID(ctx context.Context) string {
	if id, ok := ctx.Value(TenantIDKey).(string); ok {
		return id
	}
	return ""
}
