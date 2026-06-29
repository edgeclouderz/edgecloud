package handler

import (
	"context"
	"encoding/json"
	"errors"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/edgeclouderz/edge-cloud/edge-control-plane/internal/domain"
	"github.com/edgeclouderz/edge-cloud/edge-control-plane/internal/middleware"
	"github.com/edgeclouderz/edge-cloud/edge-control-plane/internal/nats"
)

// fakeSyncBuilder is the test double for syncPayloadBuilder. Returns
// whatever the test injects; default is an empty map and nil error.
type fakeSyncBuilder struct {
	apps map[string]nats.AppConfig
	err  error
}

func (f *fakeSyncBuilder) BuildFullSync(_ context.Context, _, _ string) (map[string]nats.AppConfig, error) {
	return f.apps, f.err
}

// syncHandler builds a minimal InternalHandler with just the
// dependencies the Sync endpoint touches. Other fields stay nil — the
// handler must not call them on this code path.
func syncHandler(worker *domain.Worker, workerErr error, builder *fakeSyncBuilder) *InternalHandler {
	workerSvc := &fakeWorkerSvcForSync{worker: worker, err: workerErr}
	h := NewInternalHandler(nil, workerSvc, nil, nil, nil)
	if builder != nil {
		h.SetSyncBuilder(builder)
	}
	return h
}

// withSyncJWTTenant mirrors withWorkerCtx in internal_register_test.go:
// attaches a worker-tenant-id to the request context the same way
// middleware.WorkerAuth would after validating a real JWT. Tests that
// drive the handler past the new cross-tenant authorization check
// (e.g. the populated/empty/builder-error cases that actually reach
// BuildFullSync) need this populated; tests that 4xx/5xx before that
// check (nil-builder, missing-workerID, not-found, db-error) don't.
func withSyncJWTTenant(r *http.Request, tenantID string) *http.Request {
	return r.WithContext(context.WithValue(r.Context(), middleware.WorkerTenantIDKey, tenantID))
}

// fakeWorkerSvcForSync is a separate type from the RegisterWorker
// fakeWorkerSvc because the Sync endpoint needs Get to return a
// configurable worker. We can't extend fakeWorkerSvc with a per-test
// `worker` field without breaking the RegisterWorker tests' assumption
// that Get always returns (nil, nil).
type fakeWorkerSvcForSync struct {
	worker *domain.Worker
	err    error
}

func (f *fakeWorkerSvcForSync) Register(_ context.Context, _ string, _ *domain.RegisterWorkerRequest) error {
	return nil
}
func (f *fakeWorkerSvcForSync) ListByTenant(_ context.Context, _ string) ([]domain.Worker, error) {
	return nil, nil
}
func (f *fakeWorkerSvcForSync) Get(_ context.Context, _ string) (*domain.Worker, error) {
	return f.worker, f.err
}

// --- Sync ---------------------------------------------------------

func TestSync_NilBuilder_Returns501(t *testing.T) {
	h := syncHandler(&domain.Worker{ID: "w_1", TenantID: "t_1", Region: "global"}, nil, nil)
	req := httptest.NewRequest(http.MethodGet, "/api/internal/workers/w_1/sync", nil)
	rr := httptest.NewRecorder()

	h.Sync(rr, req)

	if rr.Code != http.StatusNotImplemented {
		t.Errorf("status=%d, want 501 (nil builder = endpoint disabled)", rr.Code)
	}
}

func TestSync_MissingWorkerID_Returns400(t *testing.T) {
	// Path param missing — handled by the mux normally, but the
	// handler must also defend against empty input.
	h := syncHandler(&domain.Worker{}, nil, &fakeSyncBuilder{})
	req := httptest.NewRequest(http.MethodGet, "/api/internal/workers//sync", nil)
	// Simulate empty path value.
	req.SetPathValue("workerID", "")
	rr := httptest.NewRecorder()

	h.Sync(rr, req)

	if rr.Code != http.StatusBadRequest {
		t.Errorf("status=%d, want 400 (missing worker_id)", rr.Code)
	}
}

func TestSync_WorkerNotFound_Returns404(t *testing.T) {
	// workerSvc.Get returns (nil, nil) when no row exists (mirrors the
	// (nil, nil) contract on Repository.GetByID). Handler maps that to
	// 404, not 500, so the worker can distinguish "deleted me" from
	// "db is on fire".
	h := syncHandler(nil, nil, &fakeSyncBuilder{})
	req := httptest.NewRequest(http.MethodGet, "/api/internal/workers/w_missing/sync", nil)
	req.SetPathValue("workerID", "w_missing")
	rr := httptest.NewRecorder()

	h.Sync(rr, req)

	if rr.Code != http.StatusNotFound {
		t.Errorf("status=%d, want 404 (unknown worker)", rr.Code)
	}
}

func TestSync_WorkerSvcError_Returns500(t *testing.T) {
	h := syncHandler(nil, errors.New("db boom"), &fakeSyncBuilder{})
	req := httptest.NewRequest(http.MethodGet, "/api/internal/workers/w_1/sync", nil)
	req.SetPathValue("workerID", "w_1")
	rr := httptest.NewRecorder()

	h.Sync(rr, req)

	if rr.Code != http.StatusInternalServerError {
		t.Errorf("status=%d, want 500 (db error)", rr.Code)
	}
}

func TestSync_BuilderError_Returns500(t *testing.T) {
	h := syncHandler(&domain.Worker{ID: "w_1", TenantID: "t_1", Region: "global"}, nil, &fakeSyncBuilder{err: errors.New("repo gone")})
	req := httptest.NewRequest(http.MethodGet, "/api/internal/workers/w_1/sync", nil)
	req.SetPathValue("workerID", "w_1")
	req = withSyncJWTTenant(req, "t_1")
	rr := httptest.NewRecorder()

	h.Sync(rr, req)

	if rr.Code != http.StatusInternalServerError {
		t.Errorf("status=%d, want 500 (builder error)", rr.Code)
	}
}

func TestSync_EmptyApps_ReturnsFullSyncEnvelopeWithEmptyMap(t *testing.T) {
	// A worker with no active deployments must still get a valid
	// response — empty apps map, NOT null. The worker's deserializer
	// would crash on `"apps": null` because the Rust HashMap doesn't
	// represent null. The handler explicitly normalizes nil to {}.
	h := syncHandler(&domain.Worker{ID: "w_1", TenantID: "t_1", Region: "global"}, nil, &fakeSyncBuilder{apps: nil})
	req := httptest.NewRequest(http.MethodGet, "/api/internal/workers/w_1/sync", nil)
	req.SetPathValue("workerID", "w_1")
	req = withSyncJWTTenant(req, "t_1")
	rr := httptest.NewRecorder()

	h.Sync(rr, req)

	if rr.Code != http.StatusOK {
		t.Fatalf("status=%d, want 200", rr.Code)
	}

	var got map[string]interface{}
	if err := json.Unmarshal(rr.Body.Bytes(), &got); err != nil {
		t.Fatalf("decode: %v; body=%s", err, rr.Body.String())
	}
	if got["type"] != "full_sync" {
		t.Errorf("type=%v, want full_sync", got["type"])
	}
	if got["tenant_id"] != "t_1" {
		t.Errorf("tenant_id=%v, want t_1", got["tenant_id"])
	}
	apps, ok := got["apps"].(map[string]interface{})
	if !ok {
		t.Fatalf("apps=%v (%T), want JSON object", got["apps"], got["apps"])
	}
	if len(apps) != 0 {
		t.Errorf("apps len=%d, want 0 (normalized from nil)", len(apps))
	}
}

func TestSync_PopulatedApps_ReturnsPayload(t *testing.T) {
	// The worker's existing handle_task_message handler treats
	// "full_sync" as identical to "task_update" — same diff logic.
	// This test locks the wire shape so a future refactor doesn't
	// accidentally change field names or omit the type field.
	want := map[string]nats.AppConfig{
		"myapp": {
			DeploymentID:   "d_1",
			DeploymentHash: "abc",
			Env:            map[string]string{"K": "v"},
			Allowlist:      []string{"api.example.com"},
			MaxMemoryMB:    256,
		},
	}
	h := syncHandler(&domain.Worker{ID: "w_1", TenantID: "t_1", Region: "us-east"}, nil, &fakeSyncBuilder{apps: want})
	req := httptest.NewRequest(http.MethodGet, "/api/internal/workers/w_1/sync", nil)
	req.SetPathValue("workerID", "w_1")
	req = withSyncJWTTenant(req, "t_1")
	rr := httptest.NewRecorder()

	h.Sync(rr, req)

	if rr.Code != http.StatusOK {
		t.Fatalf("status=%d, want 200; body=%s", rr.Code, rr.Body.String())
	}

	var got struct {
		Type     string                     `json:"type"`
		TenantID string                     `json:"tenant_id"`
		Apps     map[string]json.RawMessage `json:"apps"`
	}
	if err := json.Unmarshal(rr.Body.Bytes(), &got); err != nil {
		t.Fatalf("decode: %v", err)
	}
	if got.Type != "full_sync" {
		t.Errorf("type=%q, want full_sync", got.Type)
	}
	if got.TenantID != "t_1" {
		t.Errorf("tenant_id=%q, want t_1", got.TenantID)
	}
	cfg, ok := got.Apps["myapp"]
	if !ok {
		t.Fatalf("apps=%v, want myapp", got.Apps)
	}
	var parsed nats.AppConfig
	if err := json.Unmarshal(cfg, &parsed); err != nil {
		t.Fatalf("myapp decode: %v", err)
	}
	if parsed.DeploymentID != "d_1" || parsed.DeploymentHash != "abc" {
		t.Errorf("parsed=%+v", parsed)
	}
}

// TestSync_WrongTenant_Returns404 pins the cross-tenant
// authorization check (review of PR #166, finding #1): a worker
// authenticated as tenant A must NOT be able to fetch /sync for a
// worker_id belonging to tenant B. Without this check, a compromised
// worker JWT could enumerate other tenant workerIDs (they follow
// the documented w_<region>_<n> prefix and are visible in
// heartbeats) and pull tenant B's full app set — deployment IDs,
// hashes, env vars, allowlists.
//
// Returns 404 (not 403) with the same body as the "worker not found"
// branch so an attacker can't enumerate workerIDs by comparing
// differential responses.
func TestSync_WrongTenant_Returns404(t *testing.T) {
	// workerSvc.Get returns a worker belonging to tenant B...
	worker := &domain.Worker{ID: "w_B", TenantID: "t_B", Region: "global"}
	h := syncHandler(worker, nil, &fakeSyncBuilder{})

	// ...but the JWT is for tenant A.
	req := httptest.NewRequest(http.MethodGet, "/api/internal/workers/w_B/sync", nil)
	req.SetPathValue("workerID", "w_B")
	req = withSyncJWTTenant(req, "t_A")
	rr := httptest.NewRecorder()

	h.Sync(rr, req)

	if rr.Code != http.StatusNotFound {
		t.Errorf("status=%d, want 404 (cross-tenant request denied as not-found)", rr.Code)
	}
	// Body matches the "worker not found" branch so an attacker can't
	// distinguish "real worker, wrong tenant" from "no such worker".
	expected := `{"error": "worker not found"}`
	if got := strings.TrimSpace(rr.Body.String()); got != expected {
		t.Errorf("body=%q, want %q", got, expected)
	}
}

// TestSync_SameTenant_Allowed pins the positive path of finding #1:
// a worker whose JWT matches the worker_id's tenant gets the
// payload. (Two tests below exercise the wrong-tenant branch and
// the unknown-worker branch; this one locks "same tenant, request
// succeeds".)
func TestSync_SameTenant_Allowed(t *testing.T) {
	want := map[string]nats.AppConfig{
		"myapp": {DeploymentID: "d_1", DeploymentHash: "abc"},
	}
	h := syncHandler(
		&domain.Worker{ID: "w_1", TenantID: "t_1", Region: "global"},
		nil, &fakeSyncBuilder{apps: want},
	)
	req := httptest.NewRequest(http.MethodGet, "/api/internal/workers/w_1/sync", nil)
	req.SetPathValue("workerID", "w_1")
	req = withSyncJWTTenant(req, "t_1")
	rr := httptest.NewRecorder()

	h.Sync(rr, req)

	if rr.Code != http.StatusOK {
		t.Fatalf("status=%d, want 200 (same tenant allowed); body=%s", rr.Code, rr.Body.String())
	}
}
