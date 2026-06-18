package handler

import (
	"bytes"
	"context"
	"io"
	"mime/multipart"
	"net/http"
	"net/http/httptest"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"

	"github.com/edgeclouderz/edge-cloud/edge-control-plane/internal/domain"
	"github.com/edgeclouderz/edge-cloud/edge-control-plane/internal/middleware"
	"github.com/edgeclouderz/edge-cloud/edge-control-plane/internal/service"
)

// mockDeploymentRepo implements service.DeploymentRepoInterface for testing.
type mockDeploymentRepo struct {
	deployments []*domain.Deployment
	createErr   error
}

func (m *mockDeploymentRepo) Create(ctx context.Context, d *domain.Deployment) error {
	if m.createErr != nil {
		return m.createErr
	}
	m.deployments = append(m.deployments, d)
	return nil
}

// mockArtifactStore implements service.ArtifactStoreInterface for testing.
type mockArtifactStore struct{}

func (m *mockArtifactStore) Save(tenantID, appName, deploymentID string, r io.Reader) error {
	return nil
}

// skipIfNoEdgeMigrate skips the test if edge-migrate is not in PATH.
func skipIfNoEdgeMigrate(t *testing.T) {
	if _, err := exec.LookPath("edge-migrate"); err != nil {
		t.Skip("edge-migrate not in PATH")
	}
}

// skipIfNoClang skips if wasi-sdk clang is not available.
func skipIfNoClang(t *testing.T) {
	if _, err := exec.LookPath(filepath.Join("/usr/local/wasi-sdk/bin", "clang")); err != nil {
		t.Skip("wasi-sdk clang not available at /usr/local/wasi-sdk/bin/clang")
	}
}

// makeMigrationReq creates a multipart POST request for /api/migrate.
func makeMigrationReq(filename, language, fileContent string) (*http.Request, error) {
	body := &bytes.Buffer{}
	writer := multipart.NewWriter(body)

	if err := writer.WriteField("filename", filename); err != nil {
		return nil, err
	}
	if err := writer.WriteField("language", language); err != nil {
		return nil, err
	}
	part, err := writer.CreateFormFile("file", filename)
	if err != nil {
		return nil, err
	}
	if _, err := part.Write([]byte(fileContent)); err != nil {
		return nil, err
	}
	if err := writer.Close(); err != nil {
		return nil, err
	}

	req := httptest.NewRequest("POST", "/api/migrate", body)
	req.Header.Set("Content-Type", writer.FormDataContentType())
	return req, nil
}

func TestMigrationHandler_Migrate_Success(t *testing.T) {
	skipIfNoEdgeMigrate(t)
	skipIfNoClang(t)

	repo := &mockDeploymentRepo{}
	store := &mockArtifactStore{}
	svc := service.NewMigrationService(repo, store, "edge-migrate", "/usr/local/wasi-sdk/bin")
	h := NewMigrationHandler(svc)

	source := `#include <stdio.h>
int main() { return 0; }`
	req, err := makeMigrationReq("hello.c", "c", source)
	if err != nil {
		t.Fatalf("makeMigrationReq: %v", err)
	}
	req = req.WithContext(middleware.WithTenantID(context.Background(), "tenant-test"))

	rr := httptest.NewRecorder()
	h.Migrate(rr, req)

	if rr.Code != http.StatusOK {
		t.Errorf("expected status 200, got: %d — body: %s", rr.Code, rr.Body.String())
	}
	if !strings.Contains(rr.Header().Get("Content-Type"), "application/json") {
		t.Errorf("expected Content-Type application/json, got: %s", rr.Header().Get("Content-Type"))
	}
	body := rr.Body.String()
	if !strings.Contains(body, `"status"`) {
		t.Errorf("expected JSON with status field, got: %s", body)
	}
}

func TestMigrationHandler_Migrate_MissingFile(t *testing.T) {
	repo := &mockDeploymentRepo{}
	store := &mockArtifactStore{}
	svc := service.NewMigrationService(repo, store, "edge-migrate", "/usr/local/wasi-sdk/bin")
	h := NewMigrationHandler(svc)

	// Build multipart without a "file" field
	body := &bytes.Buffer{}
	writer := multipart.NewWriter(body)
	if err := writer.WriteField("filename", "hello.c"); err != nil {
		t.Fatalf("WriteField: %v", err)
	}
	if err := writer.WriteField("language", "c"); err != nil {
		t.Fatalf("WriteField: %v", err)
	}
	writer.Close()

	req := httptest.NewRequest("POST", "/api/migrate", body)
	req.Header.Set("Content-Type", writer.FormDataContentType())
	req = req.WithContext(middleware.WithTenantID(context.Background(), "tenant-test"))

	rr := httptest.NewRecorder()
	h.Migrate(rr, req)

	if rr.Code != http.StatusBadRequest {
		t.Errorf("expected status 400, got: %d", rr.Code)
	}
	bodyStr := rr.Body.String()
	if !strings.Contains(bodyStr, "missing file field") {
		t.Errorf("expected 'missing file field' error, got: %s", bodyStr)
	}
}

func TestMigrationHandler_Migrate_NonC_Language(t *testing.T) {
	repo := &mockDeploymentRepo{}
	store := &mockArtifactStore{}
	svc := service.NewMigrationService(repo, store, "edge-migrate", "/usr/local/wasi-sdk/bin")
	h := NewMigrationHandler(svc)

	body := &bytes.Buffer{}
	writer := multipart.NewWriter(body)
	if err := writer.WriteField("filename", "hello.rs"); err != nil {
		t.Fatalf("WriteField: %v", err)
	}
	if err := writer.WriteField("language", "rust"); err != nil {
		t.Fatalf("WriteField: %v", err)
	}
	writer.Close()

	req := httptest.NewRequest("POST", "/api/migrate", body)
	req.Header.Set("Content-Type", writer.FormDataContentType())
	req = req.WithContext(middleware.WithTenantID(context.Background(), "tenant-test"))

	rr := httptest.NewRecorder()
	h.Migrate(rr, req)

	if rr.Code != http.StatusBadRequest {
		t.Errorf("expected status 400, got: %d", rr.Code)
	}
	bodyStr := rr.Body.String()
	if !strings.Contains(bodyStr, "only C language is supported") {
		t.Errorf("expected 'only C language is supported', got: %s", bodyStr)
	}
}

func TestMigrationHandler_Migrate_NoMultipart(t *testing.T) {
	repo := &mockDeploymentRepo{}
	store := &mockArtifactStore{}
	svc := service.NewMigrationService(repo, store, "edge-migrate", "/usr/local/wasi-sdk/bin")
	h := NewMigrationHandler(svc)

	req := httptest.NewRequest("POST", "/api/migrate", strings.NewReader("not multipart"))
	req.Header.Set("Content-Type", "text/plain")
	req = req.WithContext(middleware.WithTenantID(context.Background(), "tenant-test"))

	rr := httptest.NewRecorder()
	h.Migrate(rr, req)

	if rr.Code != http.StatusBadRequest {
		t.Errorf("expected status 400, got: %d — body: %s", rr.Code, rr.Body.String())
	}
}

func TestMigrationHandler_Migrate_MissingTenantID(t *testing.T) {
	repo := &mockDeploymentRepo{}
	store := &mockArtifactStore{}
	svc := service.NewMigrationService(repo, store, "edge-migrate", "/usr/local/wasi-sdk/bin")
	h := NewMigrationHandler(svc)

	source := `#include <stdio.h>
int main() { return 0; }`
	req, err := makeMigrationReq("hello.c", "c", source)
	if err != nil {
		t.Fatalf("makeMigrationReq: %v", err)
	}
	// No tenant ID in context — uses empty context

	rr := httptest.NewRecorder()
	h.Migrate(rr, req)

	if rr.Code != http.StatusUnauthorized {
		t.Errorf("expected status 401, got: %d", rr.Code)
	}
}

// ─────────────────────────────────────────────────────────────────────
// MigrateTree handler tests (M2.C10)
// ─────────────────────────────────────────────────────────────────────

// makeTreeReq builds a multipart POST with a `tree` JSON manifest and
// one or more `file` parts. `tenant` is set into the request context
// (mimicking middleware.GetTenantID).
func makeTreeReq(t *testing.T, appName, language, manifest string, files map[string]string) *http.Request {
	t.Helper()
	body := &bytes.Buffer{}
	w := multipart.NewWriter(body)
	if appName != "" {
		_ = w.WriteField("app_name", appName)
	}
	if language != "" {
		_ = w.WriteField("language", language)
	}
	if manifest != "" {
		_ = w.WriteField("tree", manifest)
	}
	for name, content := range files {
		fw, err := w.CreateFormFile("file", name)
		if err != nil {
			t.Fatalf("CreateFormFile: %v", err)
		}
		if _, err := fw.Write([]byte(content)); err != nil {
			t.Fatalf("write file: %v", err)
		}
	}
	if err := w.Close(); err != nil {
		t.Fatalf("close writer: %v", err)
	}
	req := httptest.NewRequest("POST", "/api/migrate-tree", body)
	req.Header.Set("Content-Type", w.FormDataContentType())
	return req
}

// withTenantID stuffs a tenant ID into the request context the way
// middleware.GetTenantID expects.
func withTenantID(req *http.Request, tenantID string) *http.Request {
	ctx := middleware.WithTenantID(req.Context(), tenantID)
	return req.WithContext(ctx)
}

func TestMigrateTree_RejectsMissingTenantID(t *testing.T) {
	svc := service.NewMigrationService(&mockDeploymentRepo{}, &mockArtifactStore{}, "edge-migrate", "/wasi-sdk")
	h := NewMigrationHandler(svc)
	req := makeTreeReq(t, "hello", "c", `{"files":["main.c"]}`, map[string]string{"main.c": "int main(){}"})
	rr := httptest.NewRecorder()
	h.MigrateTree(rr, req)
	if rr.Code != http.StatusUnauthorized {
		t.Errorf("expected 401, got %d: %s", rr.Code, rr.Body.String())
	}
}

func TestMigrateTree_RejectsBadAppName(t *testing.T) {
	svc := service.NewMigrationService(&mockDeploymentRepo{}, &mockArtifactStore{}, "edge-migrate", "/wasi-sdk")
	h := NewMigrationHandler(svc)
	for _, bad := range []string{"../traversal", "Bad-Name", "a/b", ""} {
		req := makeTreeReq(t, bad, "c", `{"files":["main.c"]}`, map[string]string{"main.c": "x"})
		req = withTenantID(req, "t_1")
		rr := httptest.NewRecorder()
		h.MigrateTree(rr, req)
		if rr.Code != http.StatusBadRequest {
			t.Errorf("app_name=%q: expected 400, got %d: %s", bad, rr.Code, rr.Body.String())
		}
	}
}

func TestMigrateTree_RejectsMissingAppName(t *testing.T) {
	svc := service.NewMigrationService(&mockDeploymentRepo{}, &mockArtifactStore{}, "edge-migrate", "/wasi-sdk")
	h := NewMigrationHandler(svc)
	// Make a request without an app_name field.
	body := &bytes.Buffer{}
	w := multipart.NewWriter(body)
	_ = w.WriteField("language", "c")
	_ = w.WriteField("tree", `{"files":["main.c"]}`)
	fw, _ := w.CreateFormFile("file", "main.c")
	fw.Write([]byte("x"))
	w.Close()
	req := httptest.NewRequest("POST", "/api/migrate-tree", body)
	req.Header.Set("Content-Type", w.FormDataContentType())
	req = withTenantID(req, "t_1")
	rr := httptest.NewRecorder()
	h.MigrateTree(rr, req)
	if rr.Code != http.StatusBadRequest {
		t.Errorf("expected 400, got %d: %s", rr.Code, rr.Body.String())
	}
}

func TestMigrateTree_RejectsNonCLanguage(t *testing.T) {
	svc := service.NewMigrationService(&mockDeploymentRepo{}, &mockArtifactStore{}, "edge-migrate", "/wasi-sdk")
	h := NewMigrationHandler(svc)
	req := makeTreeReq(t, "hello", "rust", `{"files":["main.c"]}`, map[string]string{"main.c": "x"})
	req = withTenantID(req, "t_1")
	rr := httptest.NewRecorder()
	h.MigrateTree(rr, req)
	if rr.Code != http.StatusBadRequest {
		t.Errorf("expected 400 for rust, got %d: %s", rr.Code, rr.Body.String())
	}
}

func TestMigrateTree_RejectsManifestMismatch(t *testing.T) {
	svc := service.NewMigrationService(&mockDeploymentRepo{}, &mockArtifactStore{}, "edge-migrate", "/wasi-sdk")
	h := NewMigrationHandler(svc)
	// Manifest declares 2 files, but only 1 file part.
	req := makeTreeReq(t, "hello", "c",
		`{"files":["main.c","helper.c"]}`,
		map[string]string{"main.c": "x"})
	req = withTenantID(req, "t_1")
	rr := httptest.NewRecorder()
	h.MigrateTree(rr, req)
	if rr.Code != http.StatusBadRequest {
		t.Errorf("expected 400 manifest mismatch, got %d: %s", rr.Code, rr.Body.String())
	}
}

func TestMigrateTree_RejectsPathTraversal(t *testing.T) {
	svc := service.NewMigrationService(&mockDeploymentRepo{}, &mockArtifactStore{}, "edge-migrate", "/wasi-sdk")
	h := NewMigrationHandler(svc)
	// Manifest references a path with `..`.
	req := makeTreeReq(t, "hello", "c",
		`{"files":["../etc/passwd"]}`,
		map[string]string{"passwd": "x"})
	req = withTenantID(req, "t_1")
	rr := httptest.NewRecorder()
	h.MigrateTree(rr, req)
	if rr.Code != http.StatusBadRequest {
		t.Errorf("expected 400 unsafe path, got %d: %s", rr.Code, rr.Body.String())
	}
}

func TestMigrateTree_RejectsTooManyFiles(t *testing.T) {
	svc := service.NewMigrationService(&mockDeploymentRepo{}, &mockArtifactStore{}, "edge-migrate", "/wasi-sdk")
	h := NewMigrationHandler(svc)
	// Build a manifest with maxTreeFiles+1 entries. We don't actually
	// upload that many file parts — the mismatch is caught first, so
	// we use a count over the limit in a valid manifest.
	names := make([]string, maxTreeFiles+1)
	files := make(map[string]string)
	for i := range names {
		names[i] = "f" + itoa(i) + ".c"
		files[names[i]] = "x"
	}
	// JSON marshal the names.
	json := "["
	for i, n := range names {
		if i > 0 {
			json += ","
		}
		json += "\"" + n + "\""
	}
	json += "]"
	manifest := `{"files":` + json + `}`
	req := makeTreeReq(t, "hello", "c", manifest, files)
	req = withTenantID(req, "t_1")
	rr := httptest.NewRecorder()
	h.MigrateTree(rr, req)
	if rr.Code != http.StatusBadRequest {
		t.Errorf("expected 400 too-many-files, got %d: %s", rr.Code, rr.Body.String())
	}
}

func itoa(i int) string {
	if i == 0 {
		return "0"
	}
	digits := []byte{}
	for i > 0 {
		digits = append([]byte{byte('0' + i%10)}, digits...)
		i /= 10
	}
	return string(digits)
}

func TestMigrateTree_RejectsOversizedBody(t *testing.T) {
	svc := service.NewMigrationService(&mockDeploymentRepo{}, &mockArtifactStore{}, "edge-migrate", "/wasi-sdk")
	h := NewMigrationHandler(svc)
	// Build a valid multipart body that's over the cap. We use a
	// single large file part padded past maxTreeBodyBytes.
	body := &bytes.Buffer{}
	w := multipart.NewWriter(body)
	_ = w.WriteField("app_name", "hello")
	_ = w.WriteField("language", "c")
	_ = w.WriteField("tree", `{"files":["main.c"]}`)
	fw, _ := w.CreateFormFile("file", "main.c")
	padding := make([]byte, maxTreeBodyBytes+1024)
	for i := range padding {
		padding[i] = 'a'
	}
	fw.Write(padding)
	w.Close()
	req := httptest.NewRequest("POST", "/api/migrate-tree", body)
	req.Header.Set("Content-Type", w.FormDataContentType())
	req = withTenantID(req, "t_1")
	rr := httptest.NewRecorder()
	h.MigrateTree(rr, req)
	if rr.Code != http.StatusRequestEntityTooLarge {
		t.Errorf("expected 413, got %d: %s", rr.Code, rr.Body.String())
	}
}

func TestMigrateTree_RejectsMissingTree(t *testing.T) {
	svc := service.NewMigrationService(&mockDeploymentRepo{}, &mockArtifactStore{}, "edge-migrate", "/wasi-sdk")
	h := NewMigrationHandler(svc)
	// No `tree` field, no `file` parts.
	req := makeTreeReq(t, "hello", "c", "", nil)
	req = withTenantID(req, "t_1")
	rr := httptest.NewRecorder()
	h.MigrateTree(rr, req)
	if rr.Code != http.StatusBadRequest {
		t.Errorf("expected 400 missing tree, got %d: %s", rr.Code, rr.Body.String())
	}
}

func TestMigrateTree_RejectsInvalidManifestJSON(t *testing.T) {
	svc := service.NewMigrationService(&mockDeploymentRepo{}, &mockArtifactStore{}, "edge-migrate", "/wasi-sdk")
	h := NewMigrationHandler(svc)
	req := makeTreeReq(t, "hello", "c", "not json", map[string]string{"main.c": "x"})
	req = withTenantID(req, "t_1")
	rr := httptest.NewRecorder()
	h.MigrateTree(rr, req)
	if rr.Code != http.StatusBadRequest {
		t.Errorf("expected 400 bad manifest, got %d: %s", rr.Code, rr.Body.String())
	}
}
