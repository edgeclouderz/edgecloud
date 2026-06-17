package handler

import (
	"bytes"
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/edgeclouderz/edge-cloud/edge-control-plane/internal/domain"
	"github.com/edgeclouderz/edge-cloud/edge-control-plane/internal/middleware"
	"github.com/golang-jwt/jwt/v5"
)

// -----------------------------------------------------------------------
// Mock repo — exercises the handler without a live DB. The interface is
// declared in internal_logs.go and mirrors the methods we call.
// -----------------------------------------------------------------------

type mockLogEntryRepo struct {
	insertBatchFunc func(ctx context.Context, entries []domain.LogEntry) error
	lastEntries     []domain.LogEntry
	calls           int
}

func (m *mockLogEntryRepo) InsertBatch(ctx context.Context, entries []domain.LogEntry) error {
	m.calls++
	m.lastEntries = entries
	if m.insertBatchFunc != nil {
		return m.insertBatchFunc(ctx, entries)
	}
	return nil
}

// -----------------------------------------------------------------------
// Test wiring helpers
// -----------------------------------------------------------------------

const (
	testJWTSecret = "test-secret"
	testJWTIssuer = "edgecloud"
)

// validToken mints a Worker JWT with the given tenant/worker IDs and a
// 24-hour TTL (matching production).
func validToken(t *testing.T, tenantID, workerID string) string {
	t.Helper()
	claims := &middleware.WorkerClaims{
		RegisteredClaims: jwt.RegisteredClaims{
			Issuer:    testJWTIssuer,
			ExpiresAt: jwt.NewNumericDate(time.Now().Add(24 * time.Hour)),
		},
		WorkerID: workerID,
		TenantID: tenantID,
		Apps:     []string{},
	}
	token := jwt.NewWithClaims(jwt.SigningMethodHS256, claims)
	signed, err := token.SignedString([]byte(testJWTSecret))
	if err != nil {
		t.Fatalf("failed to sign token: %v", err)
	}
	return signed
}

// newIngestLogsServer builds a handler chain with WorkerAuth + IngestLogs,
// exactly as main.go wires it. Tests send requests through this server to
// exercise auth + handler together.
func newIngestLogsServer(repo *mockLogEntryRepo) http.Handler {
	h := &InternalHandler{logEntryRepo: repo}
	mux := http.NewServeMux()
	mux.HandleFunc("POST /api/internal/logs", h.IngestLogs)
	return middleware.WorkerAuth(middleware.WorkerJWTConfig{
		Secret: testJWTSecret,
		Issuer: testJWTIssuer,
	})(mux)
}

// postLogs is a small helper that serializes entries as JSON and sends the
// request through the test server. `overrideTenant` and `overrideWorker` let
// us forge an entry with a different identity from the JWT — used to test the
// tenant/worker overwrite behavior.
func postLogs(t *testing.T, server http.Handler, token string, entries []domain.LogEntry, overrideTenant, overrideWorker string) *httptest.ResponseRecorder {
	t.Helper()
	for i := range entries {
		if overrideTenant != "" {
			entries[i].TenantID = overrideTenant
		}
		if overrideWorker != "" {
			entries[i].WorkerID = overrideWorker
		}
	}
	body, err := json.Marshal(IngestLogsRequest{Entries: entries})
	if err != nil {
		t.Fatalf("marshal: %v", err)
	}
	req := httptest.NewRequest("POST", "/api/internal/logs", bytes.NewReader(body))
	if token != "" {
		req.Header.Set("Authorization", "Bearer "+token)
	}
	req.Header.Set("Content-Type", "application/json")
	rec := httptest.NewRecorder()
	server.ServeHTTP(rec, req)
	return rec
}

// -----------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------

func TestIngestLogs_HappyPath(t *testing.T) {
	repo := &mockLogEntryRepo{}
	server := newIngestLogsServer(repo)
	token := validToken(t, "t_real", "w_fra_abc123")

	entries := []domain.LogEntry{
		{DeploymentID: "d_1", AppName: "app1", Level: "info", Message: "hello", Labels: json.RawMessage(`{"k":"v"}`)},
		{DeploymentID: "d_1", AppName: "app1", Level: "warn", Message: "uh oh"},
		{DeploymentID: "d_2", AppName: "app2", Level: "error", Message: "boom"},
	}

	rec := postLogs(t, server, token, entries, "", "")
	if rec.Code != http.StatusNoContent {
		t.Fatalf("status = %d, want 204; body=%s", rec.Code, rec.Body.String())
	}
	if repo.calls != 1 {
		t.Fatalf("repo called %d times, want 1", repo.calls)
	}
	if got := len(repo.lastEntries); got != 3 {
		t.Fatalf("repo got %d entries, want 3", got)
	}
	// Auth fields overwritten from JWT.
	for i, e := range repo.lastEntries {
		if e.TenantID != "t_real" {
			t.Errorf("entry[%d].TenantID = %q, want t_real", i, e.TenantID)
		}
		if e.WorkerID != "w_fra_abc123" {
			t.Errorf("entry[%d].WorkerID = %q, want w_fra_abc123", i, e.WorkerID)
		}
	}
}

func TestIngestLogs_EmptyEntries(t *testing.T) {
	repo := &mockLogEntryRepo{}
	server := newIngestLogsServer(repo)
	token := validToken(t, "t_real", "w_fra_abc123")

	rec := postLogs(t, server, token, []domain.LogEntry{}, "", "")
	if rec.Code != http.StatusNoContent {
		t.Fatalf("status = %d, want 204", rec.Code)
	}
	if repo.calls != 0 {
		t.Errorf("repo should not be called on empty entries, got %d calls", repo.calls)
	}
}

func TestIngestLogs_EmptyBody(t *testing.T) {
	repo := &mockLogEntryRepo{}
	server := newIngestLogsServer(repo)
	token := validToken(t, "t_real", "w_fra_abc123")

	req := httptest.NewRequest("POST", "/api/internal/logs", bytes.NewReader(nil))
	req.Header.Set("Authorization", "Bearer "+token)
	req.Header.Set("Content-Type", "application/json")
	rec := httptest.NewRecorder()
	server.ServeHTTP(rec, req)

	if rec.Code != http.StatusBadRequest {
		t.Fatalf("status = %d, want 400", rec.Code)
	}
	if repo.calls != 0 {
		t.Errorf("repo should not be called on empty body, got %d calls", repo.calls)
	}
}

func TestIngestLogs_AuthMissing(t *testing.T) {
	repo := &mockLogEntryRepo{}
	server := newIngestLogsServer(repo)

	// No Authorization header — WorkerAuth should reject before the handler runs.
	rec := postLogs(t, server, "", []domain.LogEntry{{AppName: "x"}}, "", "")
	if rec.Code != http.StatusUnauthorized {
		t.Fatalf("status = %d, want 401", rec.Code)
	}
	if repo.calls != 0 {
		t.Errorf("repo should not be called when unauthenticated, got %d calls", repo.calls)
	}
}

func TestIngestLogs_AuthInvalid(t *testing.T) {
	repo := &mockLogEntryRepo{}
	server := newIngestLogsServer(repo)

	// Sign with a different secret than the server expects.
	bad := validTokenWithSecret(t, "t_real", "w_fra_abc123", "wrong-secret")
	rec := postLogs(t, server, bad, []domain.LogEntry{{AppName: "x"}}, "", "")
	if rec.Code != http.StatusUnauthorized {
		t.Fatalf("status = %d, want 401", rec.Code)
	}
	if repo.calls != 0 {
		t.Errorf("repo should not be called with bad JWT, got %d calls", repo.calls)
	}
}

func TestIngestLogs_BatchTooLarge(t *testing.T) {
	repo := &mockLogEntryRepo{}
	server := newIngestLogsServer(repo)
	token := validToken(t, "t_real", "w_fra_abc123")

	// Build a JSON body whose total size exceeds MaxLogBatchSize+1. The
	// JSON prefix is valid so the decoder actually tries to read past the
	// cap; if we sent raw garbage the test would hit a JSON syntax error
	// instead and never reach the size check.
	prefix := []byte(`{"entries":[{"message":"`)
	padding := bytes.Repeat([]byte("x"), MaxLogBatchSize+1)
	suffix := []byte(`"}]}`)
	body := append(append(prefix, padding...), suffix...)

	req := httptest.NewRequest("POST", "/api/internal/logs", bytes.NewReader(body))
	req.Header.Set("Authorization", "Bearer "+token)
	req.Header.Set("Content-Type", "application/json")
	rec := httptest.NewRecorder()
	server.ServeHTTP(rec, req)

	if rec.Code != http.StatusBadRequest {
		t.Fatalf("status = %d, want 400; body=%s", rec.Code, rec.Body.String())
	}
	if !strings.Contains(rec.Body.String(), "batch too large") {
		t.Errorf("expected error to mention 'batch too large', got: %s", rec.Body.String())
	}
	if repo.calls != 0 {
		t.Errorf("repo should not be called on oversize batch, got %d calls", repo.calls)
	}
}

func TestIngestLogs_RepoError(t *testing.T) {
	repo := &mockLogEntryRepo{
		insertBatchFunc: func(ctx context.Context, entries []domain.LogEntry) error {
			return context.DeadlineExceeded
		},
	}
	server := newIngestLogsServer(repo)
	token := validToken(t, "t_real", "w_fra_abc123")

	rec := postLogs(t, server, token, []domain.LogEntry{{AppName: "x"}}, "", "")
	if rec.Code != http.StatusInternalServerError {
		t.Fatalf("status = %d, want 500", rec.Code)
	}
	if !strings.Contains(rec.Body.String(), "internal error") {
		t.Errorf("expected error to mention 'internal error', got: %s", rec.Body.String())
	}
}

// This is the security-critical test: a worker lies about tenant_id in the
// body. The handler MUST overwrite it with the JWT's tenant_id before
// inserting. If this test ever fails, the security boundary is broken.
func TestIngestLogs_TenantIDAndWorkerOverwritten(t *testing.T) {
	repo := &mockLogEntryRepo{}
	server := newIngestLogsServer(repo)
	token := validToken(t, "t_real", "w_fra_real")

	// Body tries to attribute the log to a different tenant and a different
	// worker. The handler must refuse these and use the JWT identity.
	entries := []domain.LogEntry{{AppName: "x", Message: "leak attempt"}}
	rec := postLogs(t, server, token, entries, "t_evil", "w_fra_evil")

	if rec.Code != http.StatusNoContent {
		t.Fatalf("status = %d, want 204; body=%s", rec.Code, rec.Body.String())
	}
	if repo.calls != 1 {
		t.Fatalf("repo called %d times, want 1", repo.calls)
	}
	got := repo.lastEntries[0]
	if got.TenantID != "t_real" {
		t.Errorf("TenantID = %q, want t_real (body value t_evil must be overwritten)", got.TenantID)
	}
	if got.WorkerID != "w_fra_real" {
		t.Errorf("WorkerID = %q, want w_fra_real (body value w_fra_evil must be overwritten)", got.WorkerID)
	}
}

// validTokenWithSecret is a variant that lets a test sign with a non-default
// secret (for the AuthInvalid test).
func validTokenWithSecret(t *testing.T, tenantID, workerID, secret string) string {
	t.Helper()
	claims := &middleware.WorkerClaims{
		RegisteredClaims: jwt.RegisteredClaims{
			Issuer:    testJWTIssuer,
			ExpiresAt: jwt.NewNumericDate(time.Now().Add(24 * time.Hour)),
		},
		WorkerID: workerID,
		TenantID: tenantID,
		Apps:     []string{},
	}
	token := jwt.NewWithClaims(jwt.SigningMethodHS256, claims)
	signed, err := token.SignedString([]byte(secret))
	if err != nil {
		t.Fatalf("failed to sign token: %v", err)
	}
	return signed
}
