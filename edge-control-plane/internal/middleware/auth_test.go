package middleware

import (
	"net/http"
	"net/http/httptest"
	"testing"
)

// These tests cover only the request-parsing paths in AuthMiddleware that
// don't require a service. The actual key verification path is exercised
// end-to-end in internal/service/api_key_test.go via AuthenticateRawKey.
func TestAuthMiddleware_MissingHeader(t *testing.T) {
	m := NewAuthMiddleware(nil) // service is not invoked on this path
	handler := m.Authenticate(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		t.Fatal("next handler should not be called")
	}))

	req := httptest.NewRequest("GET", "/api/apps", nil)
	rec := httptest.NewRecorder()
	handler.ServeHTTP(rec, req)

	if rec.Code != http.StatusUnauthorized {
		t.Errorf("status = %d, want %d", rec.Code, http.StatusUnauthorized)
	}
}

func TestAuthMiddleware_InvalidFormat(t *testing.T) {
	m := NewAuthMiddleware(nil)
	handler := m.Authenticate(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		t.Fatal("next handler should not be called")
	}))

	cases := []struct {
		name string
		auth string
	}{
		{"no bearer prefix", "abcdef1234"},
		{"basic instead of bearer", "Basic abcdef1234"},
		{"only header name", "Bearer"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			req := httptest.NewRequest("GET", "/api/apps", nil)
			req.Header.Set("Authorization", tc.auth)
			rec := httptest.NewRecorder()
			handler.ServeHTTP(rec, req)
			if rec.Code != http.StatusUnauthorized {
				t.Errorf("status = %d, want %d", rec.Code, http.StatusUnauthorized)
			}
		})
	}
}
