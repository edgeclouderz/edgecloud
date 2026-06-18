package service

import (
	"context"
	"errors"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"testing"

	"github.com/edgeclouderz/edge-cloud/edge-control-plane/internal/domain"
)

// mockDeploymentRepo implements DeploymentRepoInterface for testing.
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

// mockArtifactStore implements ArtifactStoreInterface for testing.
type mockArtifactStore struct {
	artifacts map[string][]byte // key: "tenantID/appName/depID"
}

func newMockArtifactStore() *mockArtifactStore {
	return &mockArtifactStore{artifacts: make(map[string][]byte)}
}

func (m *mockArtifactStore) Save(tenantID, appName, deploymentID string, r io.Reader) error {
	data, err := io.ReadAll(r)
	if err != nil {
		return err
	}
	m.artifacts[tenantID+"/"+appName+"/"+deploymentID] = data
	return nil
}

// migrationSvcForTest builds a MigrationService with mock dependencies.
func migrationSvcForTest(repo *mockDeploymentRepo, store *mockArtifactStore) *MigrationService {
	return NewMigrationService(repo, store, "edge-migrate", "/usr/local/wasi-sdk/bin")
}

func skipIfNoEdgeMigrate(t *testing.T) {
	if _, err := exec.LookPath("edge-migrate"); err != nil {
		t.Skip("edge-migrate not in PATH")
	}
}

func skipIfNoClang(t *testing.T) {
	if _, err := exec.LookPath(filepath.Join("/usr/local/wasi-sdk/bin", "clang")); err != nil {
		t.Skip("wasi-sdk clang not available at /usr/local/wasi-sdk/bin/clang")
	}
}

// posixHTTPSource is a simple POSIX C program with socket + bind + listen + accept.
const posixHTTPSource = `#include <stdio.h>
int main() {
    int fd = socket(AF_INET, SOCK_STREAM, 0);
    bind(fd, (struct sockaddr*)&addr, sizeof(addr));
    listen(fd, 128);
    int client = accept(fd, NULL, NULL);
    return 0;
}`

// emptySource has no POSIX patterns.
const emptySource = `#include <stdio.h>
int main() {
    printf("Hello, world!\n");
    return 0;
}`

func TestMigrationService_Migrate_Success(t *testing.T) {
	skipIfNoEdgeMigrate(t)
	skipIfNoClang(t)

	repo := &mockDeploymentRepo{}
	store := newMockArtifactStore()
	svc := migrationSvcForTest(repo, store)

	report, err := svc.Migrate(context.Background(), "tenant-1", "hello.c", "c", posixHTTPSource)
	if err != nil {
		t.Fatalf("expected no error, got: %v", err)
	}

	if report.Status != domain.MigrationStatusSuccess {
		t.Errorf("expected status success, got: %s", report.Status)
	}
	if !report.WasmStored {
		t.Error("expected WasmStored=true")
	}
	if report.DeploymentID == nil || *report.DeploymentID == "" {
		t.Error("expected non-empty deployment ID")
	}
	if report.AppName != "hello" {
		t.Errorf("expected appName=hello, got: %s", report.AppName)
	}
	if len(repo.deployments) != 1 {
		t.Errorf("expected 1 deployment created, got: %d", len(repo.deployments))
	}
	if repo.deployments[0].Status != "migrated" {
		t.Errorf("expected deployment status=migrated, got: %s", repo.deployments[0].Status)
	}
	if len(store.artifacts) != 1 {
		t.Errorf("expected 1 artifact saved, got: %d", len(store.artifacts))
	}
}

func TestMigrationService_Migrate_AppNameStripsC(t *testing.T) {
	skipIfNoEdgeMigrate(t)
	skipIfNoClang(t)

	repo := &mockDeploymentRepo{}
	store := newMockArtifactStore()
	svc := migrationSvcForTest(repo, store)

	report, err := svc.Migrate(context.Background(), "tenant-1", "my_app.c", "c", emptySource)
	if err != nil {
		t.Fatalf("expected no error, got: %v", err)
	}
	if report.AppName != "my_app" {
		t.Errorf("expected appName=my_app, got: %s", report.AppName)
	}
}

func TestMigrationService_Migrate_EmptySource(t *testing.T) {
	skipIfNoEdgeMigrate(t)
	skipIfNoClang(t)

	repo := &mockDeploymentRepo{}
	store := newMockArtifactStore()
	svc := migrationSvcForTest(repo, store)

	report, err := svc.Migrate(context.Background(), "tenant-1", "hello.c", "c", emptySource)
	if err != nil {
		t.Fatalf("expected no error, got: %v", err)
	}
	if report.Status != domain.MigrationStatusSuccess {
		t.Errorf("expected status success, got: %s", report.Status)
	}
	if !report.WasmStored {
		t.Error("expected WasmStored=true")
	}
}

func TestMigrationService_Migrate_EdgeMigrateFails(t *testing.T) {
	skipIfNoEdgeMigrate(t)

	repo := &mockDeploymentRepo{}
	store := newMockArtifactStore()
	svc := NewMigrationService(repo, store, "edge-migrate-that-does-not-exist", "/usr/local/wasi-sdk/bin")

	report, err := svc.Migrate(context.Background(), "tenant-1", "hello.c", "c", posixHTTPSource)
	if !errors.Is(err, ErrEdgeMigrateFailed) {
		t.Fatalf("expected ErrEdgeMigrateFailed, got: %v", err)
	}
	if report == nil {
		t.Fatal("expected non-nil report")
	}
	if report.Status != domain.MigrationStatusFailed {
		t.Errorf("expected status failed, got: %s", report.Status)
	}
	if report.WasmStored {
		t.Error("expected WasmStored=false")
	}
	if len(repo.deployments) != 0 {
		t.Errorf("expected 0 deployments, got: %d", len(repo.deployments))
	}
}

func TestMigrationService_Migrate_ClangFails(t *testing.T) {
	skipIfNoEdgeMigrate(t)
	skipIfNoClang(t)

	repo := &mockDeploymentRepo{}
	store := newMockArtifactStore()
	svc := migrationSvcForTest(repo, store)

	// Source that edge-migrate will accept but clang will reject (syntax error)
	badSource := `int main() { invalid syntax here }`

	report, err := svc.Migrate(context.Background(), "tenant-1", "hello.c", "c", badSource)
	if !errors.Is(err, ErrClangFailed) {
		t.Fatalf("expected ErrClangFailed, got: %v", err)
	}
	if report == nil {
		t.Fatal("expected non-nil report")
	}
	if report.Status != domain.MigrationStatusPartial {
		t.Errorf("expected status partial, got: %s", report.Status)
	}
	if report.WasmStored {
		t.Error("expected WasmStored=false")
	}
	if len(report.Errors) == 0 {
		t.Error("expected at least one error in report")
	}
}

func TestMigrationService_Migrate_DBError(t *testing.T) {
	skipIfNoEdgeMigrate(t)
	skipIfNoClang(t)

	repo := &mockDeploymentRepo{createErr: os.ErrPermission}
	store := newMockArtifactStore()
	svc := migrationSvcForTest(repo, store)

	_, err := svc.Migrate(context.Background(), "tenant-1", "hello.c", "c", emptySource)
	if err == nil {
		t.Fatal("expected error when DB create fails")
	}
}

func TestMigrationService_Migrate_AppNameNoExtension(t *testing.T) {
	skipIfNoEdgeMigrate(t)
	skipIfNoClang(t)

	repo := &mockDeploymentRepo{}
	store := newMockArtifactStore()
	svc := migrationSvcForTest(repo, store)

	report, err := svc.Migrate(context.Background(), "tenant-1", "hello", "c", emptySource)
	if err != nil {
		t.Fatalf("expected no error, got: %v", err)
	}
	// filename without .c suffix should be used as-is
	if report.AppName != "hello" {
		t.Errorf("expected appName=hello, got: %s", report.AppName)
	}
}

func TestDetectTransformedPatterns(t *testing.T) {
	tests := []struct {
		name     string
		wasiC    string
		expected int // minimum number of patterns we expect to detect
	}{
		{"socket only", `wasi_socket_tcp_create`, 1},
		{"full pipeline", `#include <wasi/sockets.h>
wasi_socket_tcp_create(IP_ADDRESS_FAMILY_IPV4);
wasi_socket_tcp_start_bind(fd, addr);
wasi_socket_tcp_start_listen(fd, 128);
wasi_socket_tcp_accept(fd);`, 4},
		{"empty", ``, 0},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			patterns := detectTransformedPatterns(tt.wasiC)
			if len(patterns) < tt.expected {
				t.Errorf("detectTransformedPatterns() returned %d patterns, want at least %d", len(patterns), tt.expected)
			}
		})
	}
}

func TestValidateWasm(t *testing.T) {
	tests := []struct {
		name  string
		data  []byte
		valid bool
	}{
		{"valid wasm magic", []byte{0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00}, true},
		{"empty", []byte{}, false},
		{"wrong magic", []byte{0x00, 0x00, 0x00, 0x00}, false},
		{"partial magic", []byte{0x00, 0x61, 0x73}, false},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if got := validateWasm(tt.data); got != tt.valid {
				t.Errorf("ValidateWasm() = %v, want %v", got, tt.valid)
			}
		})
	}
}

func TestIsValidDeploymentAppName(t *testing.T) {
	tests := []struct {
		name  string
		input string
		want  bool
	}{
		// Valid
		{"single char", "a", true},
		{"alphanumeric", "hello", true},
		{"with hyphen", "hello-world", true},
		{"trailing digit", "app123", true},
		{"starts with digit", "0app", true},
		{"63 chars", "a" + repeat("b", 62), true},
		// Invalid
		{"empty", "", false},
		{"64 chars", "a" + repeat("b", 63), false},
		{"uppercase", "Hello", false},
		{"all uppercase", "HELLO", false},
		{"starts with hyphen", "-hello", false},
		{"underscore", "hello_world", false},
		{"dot", "hello.world", false},
		{"slash", "hello/world", false},
		{"space", "hello world", false},
		{"path traversal", "../traversal", false},
		{"path with bad segment", "a/../b", false},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if got := IsValidDeploymentAppName(tt.input); got != tt.want {
				t.Errorf("IsValidDeploymentAppName(%q) = %v, want %v", tt.input, got, tt.want)
			}
		})
	}
}

func repeat(s string, n int) string {
	out := make([]byte, 0, len(s)*n)
	for i := 0; i < n; i++ {
		out = append(out, s...)
	}
	return string(out)
}

func TestClassifyFromPatterns(t *testing.T) {
	auto := domain.PatternInfo{Transformability: "AutoTransformable"}
	manual := domain.PatternInfo{Transformability: "NotTransformable"}
	tests := []struct {
		name     string
		patterns []domain.PatternInfo
		want     domain.MigrationStatus
	}{
		{"empty is success", nil, domain.MigrationStatusSuccess},
		{"all auto", []domain.PatternInfo{auto, auto}, domain.MigrationStatusSuccess},
		{"only manual is failed", []domain.PatternInfo{manual}, domain.MigrationStatusFailed},
		{"mixed is partial", []domain.PatternInfo{auto, manual}, domain.MigrationStatusPartial},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if got := classifyFromPatterns(tt.patterns); got != tt.want {
				t.Errorf("classifyFromPatterns() = %v, want %v", got, tt.want)
			}
		})
	}
}

func TestAggregateTreeStatus(t *testing.T) {
	mk := func(s domain.MigrationStatus) domain.FileReport {
		return domain.FileReport{Path: "x.c", Status: s}
	}
	tests := []struct {
		name  string
		files []domain.FileReport
		want  domain.MigrationStatus
	}{
		{"empty is success", nil, domain.MigrationStatusSuccess},
		{"all success", []domain.FileReport{mk(domain.MigrationStatusSuccess), mk(domain.MigrationStatusSuccess)}, domain.MigrationStatusSuccess},
		{"one partial", []domain.FileReport{mk(domain.MigrationStatusSuccess), mk(domain.MigrationStatusPartial)}, domain.MigrationStatusPartial},
		{"any failed wins", []domain.FileReport{mk(domain.MigrationStatusSuccess), mk(domain.MigrationStatusPartial), mk(domain.MigrationStatusFailed)}, domain.MigrationStatusFailed},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if got := aggregateTreeStatus(tt.files); got != tt.want {
				t.Errorf("aggregateTreeStatus() = %v, want %v", got, tt.want)
			}
		})
	}
}

func TestMigrateTree_RejectsInvalidAppName(t *testing.T) {
	svc := migrationSvcForTest(&mockDeploymentRepo{}, newMockArtifactStore())
	_, err := svc.MigrateTree(context.Background(), "t_1", "../bad", "c", []domain.FileEntry{
		{Path: "main.c", Source: "int main(){return 0;}\n"},
	})
	if err == nil {
		t.Fatal("expected error for invalid app name")
	}
}

func TestMigrateTree_RejectsEmptyTree(t *testing.T) {
	svc := migrationSvcForTest(&mockDeploymentRepo{}, newMockArtifactStore())
	_, err := svc.MigrateTree(context.Background(), "t_1", "hello", "c", nil)
	if err == nil {
		t.Fatal("expected error for empty tree")
	}
}

func TestMigrateTree_RejectsUnsupportedLanguage(t *testing.T) {
	svc := migrationSvcForTest(&mockDeploymentRepo{}, newMockArtifactStore())
	_, err := svc.MigrateTree(context.Background(), "t_1", "hello", "rust", []domain.FileEntry{
		{Path: "main.rs", Source: "fn main() {}\n"},
	})
	if err == nil {
		t.Fatal("expected error for unsupported language")
	}
}

func TestMigrateTree_RejectsPathTraversal(t *testing.T) {
	svc := migrationSvcForTest(&mockDeploymentRepo{}, newMockArtifactStore())
	_, err := svc.MigrateTree(context.Background(), "t_1", "hello", "c", []domain.FileEntry{
		{Path: "../etc/passwd", Source: "x"},
	})
	if err == nil {
		t.Fatal("expected error for path traversal")
	}
}
