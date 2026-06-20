package handler

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/edgeclouderz/edge-cloud/edge-control-plane/internal/middleware"
	"github.com/edgeclouderz/edge-cloud/edge-control-plane/internal/service"
)

// stubActivator is the minimum implementation of deploymentActivator
// needed by the Activate handler. It records the args it was called
// with so tests can assert the tenant filter is applied correctly,
// and returns whatever response/err the test sets.
type stubActivator struct {
	err    error
	called bool
	// lastTenant / lastApp / lastDeploymentID record the arguments the
	// handler passed so tests can assert that the tenant context (not
	// the URL) wins and that the path values reach the service layer.
	lastTenant       string
	lastApp          string
	lastDeploymentID string
}

func (s *stubActivator) ActivateDeployment(_ context.Context, tenantID, appName, deploymentID string) error {
	s.called = true
	s.lastTenant = tenantID
	s.lastApp = appName
	s.lastDeploymentID = deploymentID
	return s.err
}

// newActivateMux wires a single POST /api/apps/{appName}/activate/{deploymentID}
// route through a real *http.ServeMux so r.PathValue("appName") and
// r.PathValue("deploymentID") are populated the same way they are in
// production. workerSvc, deploymentSvc, and rollbackSvc are nil because
// the Activate body never touches them.
func newActivateMux(svc *stubActivator) *http.ServeMux {
	h := &DeploymentHandler{
		workerSvc:     nil,
		deploymentSvc: nil,
		rollbackSvc:   nil,
		activateSvc:   svc,
	}
	mux := http.NewServeMux()
	mux.HandleFunc("POST /api/apps/{appName}/activate/{deploymentID}", h.Activate)
	return mux
}

// ---------------------------------------------------------------------------
// Activate — 200 (happy path)
// ---------------------------------------------------------------------------

func TestActivate_HappyPath_Returns200(t *testing.T) {
	svc := &stubActivator{}
	mux := newActivateMux(svc)

	req := httptest.NewRequest("POST", "/api/apps/myapp/activate/d_x", nil)
	req = req.WithContext(middleware.WithTenantID(req.Context(), "t_test"))
	rr := httptest.NewRecorder()
	mux.ServeHTTP(rr, req)

	if rr.Code != http.StatusOK {
		t.Fatalf("status = %d, want 200; body: %s", rr.Code, rr.Body.String())
	}
	if ct := rr.Header().Get("Content-Type"); ct != "application/json" {
		t.Errorf("Content-Type = %q, want application/json", ct)
	}
	var got map[string]string
	if err := json.Unmarshal(rr.Body.Bytes(), &got); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	if got["status"] != "activated" {
		t.Errorf("status = %q, want activated", got["status"])
	}
	// The handler must propagate the tenant id from the auth context,
	// not from the URL — this is what keeps cross-tenant activation
	// from working.
	if !svc.called {
		t.Fatal("ActivateDeployment was not called")
	}
	if svc.lastTenant != "t_test" {
		t.Errorf("ActivateDeployment called with tenant %q, want t_test", svc.lastTenant)
	}
	if svc.lastApp != "myapp" {
		t.Errorf("ActivateDeployment called with app %q, want myapp", svc.lastApp)
	}
	if svc.lastDeploymentID != "d_x" {
		t.Errorf("ActivateDeployment called with deploymentID %q, want d_x", svc.lastDeploymentID)
	}
}

// ---------------------------------------------------------------------------
// Activate — 502 (post-commit NATS publish failed)
// ---------------------------------------------------------------------------

func TestActivate_PublishFailed_Returns502(t *testing.T) {
	// Service returns the wrapped ErrPublishFailed sentinel that
	// ActivateDeployment emits when PublishTaskUpdate fails after the
	// DB transaction has committed. Handler must surface this as 502
	// (not 500) so the client knows the DB write may have succeeded
	// and to treat it as an upstream-dependency failure.
	wrapped := fmt.Errorf("%w: %w", service.ErrPublishFailed, errors.New("nats unreachable"))
	svc := &stubActivator{err: wrapped}
	mux := newActivateMux(svc)

	req := httptest.NewRequest("POST", "/api/apps/myapp/activate/d_x", nil)
	req = req.WithContext(middleware.WithTenantID(req.Context(), "t_test"))
	rr := httptest.NewRecorder()
	mux.ServeHTTP(rr, req)

	if rr.Code != http.StatusBadGateway {
		t.Fatalf("status = %d, want 502; body: %s", rr.Code, rr.Body.String())
	}
	if !strings.Contains(rr.Body.String(), "worker notification failed") {
		t.Errorf("body should explain 502, got %s", rr.Body.String())
	}
	// Body must not leak the sentinel or the raw NATS error.
	if strings.Contains(rr.Body.String(), "nats unreachable") {
		t.Errorf("body leaks raw error: %s", rr.Body.String())
	}
	if strings.Contains(rr.Body.String(), "ErrPublishFailed") {
		t.Errorf("body leaks sentinel: %s", rr.Body.String())
	}
}

// ---------------------------------------------------------------------------
// Activate — 500 (unexpected service error)
// ---------------------------------------------------------------------------

func TestActivate_ServiceError_Returns500(t *testing.T) {
	svc := &stubActivator{err: errors.New("db unreachable")}
	mux := newActivateMux(svc)

	req := httptest.NewRequest("POST", "/api/apps/myapp/activate/d_x", nil)
	req = req.WithContext(middleware.WithTenantID(req.Context(), "t_test"))
	rr := httptest.NewRecorder()
	mux.ServeHTTP(rr, req)

	if rr.Code != http.StatusInternalServerError {
		t.Fatalf("status = %d, want 500; body: %s", rr.Code, rr.Body.String())
	}
	if strings.Contains(rr.Body.String(), "db unreachable") {
		t.Errorf("body must not leak raw error, got %s", rr.Body.String())
	}
	// 502 must NOT be returned for an unrelated error.
	if rr.Code == http.StatusBadGateway {
		t.Errorf("status = 502, want 500; non-publish errors must not become 502")
	}
}
