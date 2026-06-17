package handler

import (
	"context"
	"errors"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/edgeclouderz/edge-cloud/edge-control-plane/internal/handler"
)

// mockTenantSvc implements handler.TenantServiceInterface for testing.
type mockTenantSvc struct {
	bootstrapErr error
}

func (m *mockTenantSvc) BootstrapTenant(ctx context.Context, name, plan, keyName string) (interface{}, string, error) {
	return nil, "", m.bootstrapErr
}
func (m *mockTenantSvc) CreateTenant(ctx context.Context, name, plan string) (interface{}, error) {
	return nil, nil
}
func (m *mockTenantSvc) GetTenant(ctx context.Context, id string) (interface{}, error) {
	return nil, nil
}
func (m *mockTenantSvc) ListTenants(ctx context.Context) ([]interface{}, error) {
	return nil, nil
}
func (m *mockTenantSvc) UpdateTenant(ctx context.Context, t interface{}) error {
	return nil
}
func (m *mockTenantSvc) DeleteTenant(ctx context.Context, id string) error {
	return nil
}

func TestTenantHandler_Bootstrap_ErrorPath(t *testing.T) {
	svc := &mockTenantSvc{bootstrapErr: errors.New("database connection refused")}
	h := handler.NewTenantHandler(svc)

	body := strings.NewReader(`{"name":"test","key_name":"default"}`)
	req := httptest.NewRequest("POST", "/api/tenants", body)
	req.Header.Set("Content-Type", "application/json")
	rr := httptest.NewRecorder()

	h.Bootstrap(rr, req)

	if rr.Code != http.StatusInternalServerError {
		t.Errorf("expected status 500, got: %d", rr.Code)
	}
	respBody := rr.Body.String()
	// Must NOT contain raw internal error
	if strings.Contains(respBody, "database connection refused") {
		t.Errorf("response should not contain raw error, got: %s", respBody)
	}
	if !strings.Contains(respBody, `"error"`) {
		t.Errorf("expected JSON error field, got: %s", respBody)
	}
}
