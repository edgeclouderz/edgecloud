package service

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/hex"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"strings"
	"time"

	"github.com/edgeclouderz/edge-cloud/edge-control-plane/internal/domain"
	"github.com/edgeclouderz/edge-cloud/edge-control-plane/internal/repository"
	"github.com/edgeclouderz/edge-cloud/edge-control-plane/internal/storage"
	"github.com/google/uuid"
)

type MigrationService struct {
	deploymentRepo  *repository.DeploymentRepository
	artifactStore   *storage.ArtifactStore
	edgeMigratePath string
	wasiSdkPath     string
}

func NewMigrationService(
	deploymentRepo *repository.DeploymentRepository,
	artifactStore *storage.ArtifactStore,
	edgeMigratePath, wasiSdkPath string,
) *MigrationService {
	return &MigrationService{
		deploymentRepo:  deploymentRepo,
		artifactStore:   artifactStore,
		edgeMigratePath: edgeMigratePath,
		wasiSdkPath:     wasiSdkPath,
	}
}

func (s *MigrationService) Migrate(
	ctx context.Context,
	tenantID, filename, language, source string,
) (*domain.MigrationReport, error) {
	if language != "c" {
		return nil, fmt.Errorf("only C language is supported, got: %s", language)
	}

	appName := strings.TrimSuffix(filepath.Base(filename), ".c")
	if !IsValidAppName(appName) {
		return nil, fmt.Errorf("invalid app name derived from filename: %s", appName)
	}

	patterns := detectPatterns(source)

	tmpDir := os.TempDir()
	srcPath := filepath.Join(tmpDir, fmt.Sprintf("edge_migrate_src_%d.c", time.Now().UnixNano()))
	if err := os.WriteFile(srcPath, []byte(source), 0600); err != nil {
		return nil, fmt.Errorf("writing temp source file: %w", err)
	}
	defer os.Remove(srcPath)

	wasiSource, err := s.runEdgeMigrateTransform(ctx, srcPath)
	if err != nil {
		return nil, fmt.Errorf("edge-migrate transform: %w", err)
	}

	wasmPath := filepath.Join(tmpDir, fmt.Sprintf("edge_migrate_out_%d.wasm", time.Now().UnixNano()))
	defer os.Remove(wasmPath)

	if err := s.compileWasm(ctx, wasiSource, wasmPath); err != nil {
		report := buildReport(appName, patterns, nil, false)
		report.Errors = append(report.Errors, domain.ErrorInfo{
			Message: fmt.Sprintf("compilation failed: %s", err.Error()),
		})
		report.Status = domain.MigrationStatusFailed
		return report, nil
	}

	wasmData, err := os.ReadFile(wasmPath)
	if err != nil {
		return nil, fmt.Errorf("reading compiled wasm: %w", err)
	}
	hash := sha256.Sum256(wasmData)
	deploymentID := "d_" + uuid.New().String()

	// Save artifact first so we never have a deployment record without an artifact.
	if err := s.artifactStore.Save(tenantID, appName, deploymentID, bytes.NewReader(wasmData)); err != nil {
		return nil, fmt.Errorf("saving wasm artifact: %w", err)
	}

	deployment := &domain.Deployment{
		ID:        deploymentID,
		TenantID:  tenantID,
		AppName:   appName,
		Status:    "migrated",
		Hash:      hex.EncodeToString(hash[:]),
		CreatedAt: time.Now(),
	}
	if err := s.deploymentRepo.Create(ctx, deployment); err != nil {
		return nil, fmt.Errorf("creating deployment record: %w", err)
	}

	report := buildReport(appName, patterns, &deploymentID, true)
	return report, nil
}

func (s *MigrationService) runEdgeMigrateTransform(ctx context.Context, srcPath string) (string, error) {
	cmd := exec.CommandContext(ctx, s.edgeMigratePath, "--transform", srcPath)
	var stdout, stderr bytes.Buffer
	cmd.Stdout = &stdout
	cmd.Stderr = &stderr
	if err := cmd.Run(); err != nil {
		return "", fmt.Errorf("edge-migrate failed: %s (%s)", err.Error(), stderr.String())
	}
	return stdout.String(), nil
}

func (s *MigrationService) compileWasm(ctx context.Context, wasiSource, wasmPath string) error {
	clangPath := filepath.Join(s.wasiSdkPath, "clang")
	cmd := exec.CommandContext(ctx, clangPath,
		"--target=wasm32-wasip2", "-nostdlib", "-o", wasmPath, "-",
	)
	cmd.Stdin = strings.NewReader(wasiSource)
	var stderr bytes.Buffer
	cmd.Stderr = &stderr
	if err := cmd.Run(); err != nil {
		return fmt.Errorf("clang failed: %s (%s)", err.Error(), stderr.String())
	}
	return nil
}

type patternDef struct {
	regex          *regexp.Regexp
	name           string
	wasiEquivalent string
	transformable  bool
}

var posixPatterns = []patternDef{
	{regexp.MustCompile(`\bsocket\s*\([^)]*SOCK_STREAM[^)]*\)`), "SocketTcp", "wasi_socket_tcp_create", true},
	{regexp.MustCompile(`\bsocket\s*\([^)]*SOCK_DGRAM[^)]*\)`), "SocketUdp", "wasi_socket_udp_create", true},
	{regexp.MustCompile(`\bbind\s*\(`), "Bind", "wasi_socket_tcp_start_bind / finish_bind", true},
	{regexp.MustCompile(`\blisten\s*\(`), "Listen", "wasi_socket_tcp_start_listen / finish_listen", true},
	{regexp.MustCompile(`\baccept4?\s*\(`), "Accept", "wasi_socket_tcp_accept", true},
	{regexp.MustCompile(`\bconnect\s*\(`), "Connect", "wasi_socket_tcp_start_connect / finish_connect", true},
	{regexp.MustCompile(`\brecv\s*\(`), "Recv", "wasi_input_stream_read", true},
	{regexp.MustCompile(`\bsend\s*\(`), "Send", "wasi_output_stream_write", true},
	{regexp.MustCompile(`\bgethostbyname\s*\(`), "GetHostByName", "wasi_ip_name_lookup_resolve", true},
	{regexp.MustCompile(`\bfopen\s*\(`), "Fopen", "wasi_filesystem_open", true},
	{regexp.MustCompile(`\bfread\s*\(`), "Fread", "wasi_filesystem_read", true},
	{regexp.MustCompile(`\bfwrite\s*\(`), "Fwrite", "wasi_filesystem_write", true},
	{regexp.MustCompile(`\bfclose\s*\(`), "Fclose", "wasi_filesystem_close", true},
	{regexp.MustCompile(`\bclose\s*\(`), "Close", "wasi_socket_close", true},
	{regexp.MustCompile(`\bpoll\s*\(`), "Poll", "wasi_poll_poll", false},
	{regexp.MustCompile(`\bselect\s*\(`), "Select", "", false},
	{regexp.MustCompile(`\bfork\s*\(`), "Fork", "", false},
	{regexp.MustCompile(`\bexecv?e?\s*\(`), "Exec", "", false},
}

type detectedPattern struct {
	name           string
	transformable  bool
	wasiEquivalent string
	line           int
	snippet        string
}

func detectPatterns(source string) []detectedPattern {
	lines := strings.Split(source, "\n")
	var results []detectedPattern
	for lineNum, line := range lines {
		for _, p := range posixPatterns {
			if p.regex.MatchString(line) {
				results = append(results, detectedPattern{
					name:           p.name,
					transformable:  p.transformable,
					wasiEquivalent: p.wasiEquivalent,
					line:           lineNum + 1,
					snippet:        strings.TrimSpace(line),
				})
			}
		}
	}
	return results
}

func buildReport(appName string, patterns []detectedPattern, deploymentID *string, wasmStored bool) *domain.MigrationReport {
	var detected, transformed, manualReview []domain.PatternInfo
	for _, p := range patterns {
		transformability := "NotTransformable"
		if p.transformable {
			transformability = "AutoTransformable"
		}
		info := domain.PatternInfo{
			Line:             p.line,
			Pattern:          p.name,
			Snippet:          p.snippet,
			WasiEquivalent:   p.wasiEquivalent,
			Transformability: transformability,
		}
		detected = append(detected, info)
		if p.transformable {
			transformed = append(transformed, info)
		} else {
			manualReview = append(manualReview, info)
		}
	}
	status := domain.MigrationStatusSuccess
	if len(manualReview) > 0 {
		if len(transformed) == 0 {
			status = domain.MigrationStatusFailed
		} else {
			status = domain.MigrationStatusPartial
		}
	}
	return &domain.MigrationReport{
		Status:               status,
		WasmStored:           wasmStored,
		DeploymentID:         deploymentID,
		AppName:              appName,
		PatternsDetected:     detected,
		PatternsTransformed:  transformed,
		PatternsManualReview: manualReview,
		Errors:               []domain.ErrorInfo{},
	}
}
