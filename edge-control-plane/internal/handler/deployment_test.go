package handler

import (
	"context"
	"encoding/json"
	"errors"
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/edgeclouderz/edge-cloud/edge-control-plane/internal/domain"
	"github.com/edgeclouderz/edge-cloud/edge-control-plane/internal/middleware"
)

// mockAppTargetLookup is the minimum service.AppTargetLookup implementation
// needed by AppIngress. Kept narrow so adding a method to the real
// service doesn't ripple into this test.
type mockAppTargetLookup struct {
	target *domain.AppTarget
	err    error
	// lastTenant / lastApp record the arguments the handler passed to
	// GetAppTarget so tests can assert the tenant filter is applied
	// correctly.
	lastTenant string
	lastApp    string
}

func (m *mockAppTargetLookup) GetAppTarget(_ context.Context, tenantID, appName string) (*domain.AppTarget, error) {
	m.lastTenant = tenantID
	m.lastApp = appName
	return m.target, m.err
}

// newAppIngressMux wires a single route through a real *http.ServeMux so
// r.PathValue("appName") is populated the same way it is in production.
// The deploymentSvc is nil because AppIngress never calls into it —
// the test exists to lock the AppIngress contract, not the deployment
// service contract.
func newAppIngressMux(lookup *mockAppTargetLookup) *http.ServeMux {
	mux := http.NewServeMux()
	mux.HandleFunc("GET /api/apps/{appName}/ingress", NewDeploymentHandler(nil, lookup).AppIngress)
	return mux
}

// ---------------------------------------------------------------------------
// AppIngress — 200 (found)
// ---------------------------------------------------------------------------

func TestAppIngress_Found_Returns200AndFullTarget(t *testing.T) {
	want := &domain.AppTarget{
		AppName:    "myapp",
		TenantID:   "t_test",
		WorkerID:   "w_fra_abc",
		Region:     "fra",
		WorkerAddr: "203.0.113.10",
		Port:       8081,
	}
	lookup := &mockAppTargetLookup{target: want}
	mux := newAppIngressMux(lookup)

	req := httptest.NewRequest("GET", "/api/apps/myapp/ingress", nil)
	req = req.WithContext(middleware.WithTenantID(req.Context(), "t_test"))
	rr := httptest.NewRecorder()
	mux.ServeHTTP(rr, req)

	if rr.Code != http.StatusOK {
		t.Fatalf("status = %d, want 200; body: %s", rr.Code, rr.Body.String())
	}
	if ct := rr.Header().Get("Content-Type"); ct != "application/json" {
		t.Errorf("Content-Type = %q, want application/json", ct)
	}
	var got map[string]interface{}
	if err := json.Unmarshal(rr.Body.Bytes(), &got); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	if got["ready"] != true {
		t.Errorf("ready = %v, want true", got["ready"])
	}
	if got["app_name"] != "myapp" {
		t.Errorf("app_name = %v, want myapp", got["app_name"])
	}
	if got["tenant_id"] != "t_test" {
		t.Errorf("tenant_id = %v, want t_test", got["tenant_id"])
	}
	if got["worker_addr"] != "203.0.113.10" {
		t.Errorf("worker_addr = %v, want 203.0.113.10", got["worker_addr"])
	}
	// JSON numbers decode to float64.
	if port, _ := got["port"].(float64); int(port) != 8081 {
		t.Errorf("port = %v, want 8081", got["port"])
	}
	// The handler must propagate the tenant id from the auth context, not
	// from the URL — this is what keeps cross-tenant lookup from working.
	if lookup.lastTenant != "t_test" {
		t.Errorf("GetAppTarget called with tenant %q, want t_test", lookup.lastTenant)
	}
	if lookup.lastApp != "myapp" {
		t.Errorf("GetAppTarget called with app %q, want myapp", lookup.lastApp)
	}
}

// ---------------------------------------------------------------------------
// AppIngress — 404 (no running target for this tenant)
// ---------------------------------------------------------------------------

func TestAppIngress_NotFound_Returns404AndStructuredBody(t *testing.T) {
	lookup := &mockAppTargetLookup{target: nil}
	mux := newAppIngressMux(lookup)

	req := httptest.NewRequest("GET", "/api/apps/missing/ingress", nil)
	req = req.WithContext(middleware.WithTenantID(req.Context(), "t_test"))
	rr := httptest.NewRecorder()
	mux.ServeHTTP(rr, req)

	if rr.Code != http.StatusNotFound {
		t.Fatalf("status = %d, want 404; body: %s", rr.Code, rr.Body.String())
	}
	if ct := rr.Header().Get("Content-Type"); ct != "application/json" {
		t.Errorf("Content-Type = %q, want application/json", ct)
	}
	var got map[string]interface{}
	if err := json.Unmarshal(rr.Body.Bytes(), &got); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	if got["ready"] != false {
		t.Errorf("ready = %v, want false", got["ready"])
	}
	if got["app_name"] != "missing" {
		t.Errorf("app_name = %v, want missing", got["app_name"])
	}
	if _, ok := got["reason"]; !ok {
		t.Errorf("reason field missing from 404 body: %s", rr.Body.String())
	}
}

// ---------------------------------------------------------------------------
// AppIngress — 500 (service error)
// ---------------------------------------------------------------------------

func TestAppIngress_ServiceError_Returns500(t *testing.T) {
	lookup := &mockAppTargetLookup{err: errors.New("db unreachable")}
	mux := newAppIngressMux(lookup)

	req := httptest.NewRequest("GET", "/api/apps/myapp/ingress", nil)
	req = req.WithContext(middleware.WithTenantID(req.Context(), "t_test"))
	rr := httptest.NewRecorder()
	mux.ServeHTTP(rr, req)

	if rr.Code != http.StatusInternalServerError {
		t.Fatalf("status = %d, want 500; body: %s", rr.Code, rr.Body.String())
	}
}

// ---------------------------------------------------------------------------
// AppIngress — 400 (path traversal in appName)
// ---------------------------------------------------------------------------

// TestAppIngress_PathTraversal_Returns400 exercises the 400 path the
// handler can actually trigger. Note that the Go `http.ServeMux` does
// its own path cleaning BEFORE the handler is called:
//   - `..` and `.` segments in the URL are collapsed (mux returns 307
//     to the cleaned URL — the handler never sees them).
//   - `/` inside what would be a single wildcard segment makes the
//     pattern not match at all (mux returns 404 — the handler never
//     sees them).
//
// So the handler only has to defend against inputs that survive mux
// cleaning, e.g. backslashes and percent-encoded forms that decode
// after the mux has already moved on.
func TestAppIngress_PathTraversal_Returns400(t *testing.T) {
	tests := []struct {
		name    string
		appName string // path segment as it appears in the URL
	}{
		// Backslash is a path-separator on Windows; POSIX URL parsers
		// pass it through verbatim. The handler's containsPathTraversal
		// explicitly rejects it.
		{"backslash", `foo\bar`},
		// %2E%2E reaches the handler as the literal string ".." because
		// Go's net/url decodes percent-escapes in the path before the
		// mux fills the path value (and the mux's `..` cleaning only
		// runs on the un-decoded URL — see net/http serverHandler).
		{"percent-encoded dots", "%2E%2E"},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			lookup := &mockAppTargetLookup{}
			mux := newAppIngressMux(lookup)
			url := "/api/apps/" + tt.appName + "/ingress"
			req := httptest.NewRequest("GET", url, nil)
			req = req.WithContext(middleware.WithTenantID(req.Context(), "t_test"))
			rr := httptest.NewRecorder()
			mux.ServeHTTP(rr, req)

			if rr.Code != http.StatusBadRequest {
				t.Errorf("status = %d, want 400; body: %s", rr.Code, rr.Body.String())
			}
			if lookup.lastApp != "" {
				t.Errorf("GetAppTarget should not have been called for traversal appName, got %q", lookup.lastApp)
			}
		})
	}
}
