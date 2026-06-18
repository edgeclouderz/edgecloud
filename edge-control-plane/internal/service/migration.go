package service

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"time"

	"github.com/edgeclouderz/edge-cloud/edge-control-plane/internal/domain"
	"github.com/google/uuid"
)

// ErrMigrateTreeFailed is returned when the tree-mode migration fails
// (per-file errors don't trigger this; only tree-level errors do —
// the report is still returned in the partial-success case).
var ErrMigrateTreeFailed = fmt.Errorf("tree migration failed")

// ErrEdgeMigrateFailed is returned when the edge-migrate subprocess fails.
var ErrEdgeMigrateFailed = fmt.Errorf("edge-migrate transform failed")

// ErrClangFailed is returned when the wasi-sdk clang subprocess fails.
var ErrClangFailed = fmt.Errorf("wasi-sdk clang compilation failed")

// DeploymentRepoInterface abstracts deployment creation for testing.
type DeploymentRepoInterface interface {
	Create(ctx context.Context, d *domain.Deployment) error
}

// ArtifactStoreInterface abstracts wasm artifact storage for testing.
type ArtifactStoreInterface interface {
	Save(tenantID, appName, deploymentID string, r io.Reader) error
}

// MigrationService transforms POSIX C source to WASI and compiles it to wasm.
type MigrationService struct {
	deploymentRepo  DeploymentRepoInterface
	artifactStore   ArtifactStoreInterface
	edgeMigratePath string
	wasiSdkPath     string
}

// NewMigrationService creates a MigrationService.
func NewMigrationService(
	deploymentRepo DeploymentRepoInterface,
	artifactStore ArtifactStoreInterface,
	edgeMigratePath, wasiSdkPath string,
) *MigrationService {
	return &MigrationService{
		deploymentRepo:  deploymentRepo,
		artifactStore:   artifactStore,
		edgeMigratePath: edgeMigratePath,
		wasiSdkPath:     wasiSdkPath,
	}
}

// Migrate transforms the given C source to WASI C, compiles it to wasm,
// stores the artifact, and creates a deployment record.
func (s *MigrationService) Migrate(ctx context.Context, tenantID, filename, _language, source string) (*domain.MigrationReport, error) {
	// Derive app name: strip .c suffix
	appName := strings.TrimSuffix(filename, ".c")
	if appName == "" {
		appName = "app"
	}

	// Write source to a temp file for edge-migrate (reads a path, not stdin)
	tmpSrc, err := os.CreateTemp("", "migrate-*.c")
	if err != nil {
		return nil, fmt.Errorf("creating temp source file: %w", err)
	}
	tmpSrcPath := tmpSrc.Name()
	defer os.Remove(tmpSrcPath)
	if _, err := tmpSrc.WriteString(source); err != nil {
		tmpSrc.Close()
		return nil, fmt.Errorf("writing temp source: %w", err)
	}
	tmpSrc.Close()

	// Run edge-migrate --transform <path>
	edgeMigCmd := exec.CommandContext(ctx, s.edgeMigratePath, "--transform", tmpSrcPath)
	var edgeMigOut bytes.Buffer
	edgeMigCmd.Stdout = &edgeMigOut
	var edgeMigErr bytes.Buffer
	edgeMigCmd.Stderr = &edgeMigErr
	if err := edgeMigCmd.Run(); err != nil {
		return &domain.MigrationReport{
			Status:    domain.MigrationStatusFailed,
			WasmStored: false,
			AppName:   appName,
			Errors: []domain.ErrorInfo{{
				Line:    0,
				Message: fmt.Sprintf("edge-migrate failed: %s — %s", err, edgeMigErr.String()),
			}},
		}, ErrEdgeMigrateFailed
	}
	wasiC := edgeMigOut.String()

	// Build pattern report from the WASI C output
	patternsTransformed := detectTransformedPatterns(wasiC)

	// Compile WASI C → wasm via clang
	tmpWasm, err := os.CreateTemp("", "migrate-*.wasm")
	if err != nil {
		return nil, fmt.Errorf("creating temp wasm file: %w", err)
	}
	tmpWasmPath := tmpWasm.Name()
	tmpWasm.Close()
	defer os.Remove(tmpWasmPath)

	clangBin := filepath.Join(s.wasiSdkPath, "clang")
	clangCmd := exec.CommandContext(ctx, clangBin,
		"--target=wasm32-wasip2", "-nostdlib",
		"-o", tmpWasmPath, "-")
	clangCmd.Stdin = strings.NewReader(wasiC)
	var clangErr bytes.Buffer
	clangCmd.Stderr = &clangErr

	if err := clangCmd.Run(); err != nil {
		return &domain.MigrationReport{
			Status:              domain.MigrationStatusPartial,
			WasmStored:          false,
			AppName:             appName,
			PatternsTransformed: patternsTransformed,
			Errors: []domain.ErrorInfo{{
				Line:    0,
				Message: fmt.Sprintf("clang failed: %s — %s", err, clangErr.String()),
			}},
		}, ErrClangFailed
	}

	// Read wasm bytes
	wasmBytes, err := os.ReadFile(tmpWasmPath)
	if err != nil {
		return nil, fmt.Errorf("reading compiled wasm: %w", err)
	}

	// Enforce MaxArtifactSize. Catches accidental huge builds (e.g.,
	// debug symbols left in, broken optimization) before we ever
	// hit the database or filesystem. Closes the pre-existing gap on
	// the single-file `Migrate` path (M2.C8) — MigrateTree enforces
	// the same cap separately.
	if int64(len(wasmBytes)) > MaxArtifactSize {
		return &domain.MigrationReport{
			Status:              domain.MigrationStatusFailed,
			WasmStored:          false,
			AppName:             appName,
			PatternsTransformed: patternsTransformed,
			Errors: []domain.ErrorInfo{{
				Line:    0,
				Message: fmt.Sprintf("wasm exceeds %d bytes (MaxArtifactSize)", MaxArtifactSize),
			}},
		}, nil
	}

	// Generate deployment ID and hash
	depID := "d_" + uuid.New().String()
	hash := sha256.Sum256(wasmBytes)

	// Create deployment DB record
	deployment := &domain.Deployment{
		ID:        depID,
		TenantID:  tenantID,
		AppName:   appName,
		Status:    "migrated",
		Hash:      hex.EncodeToString(hash[:]),
		CreatedAt: time.Now(),
	}
	if err := s.deploymentRepo.Create(ctx, deployment); err != nil {
		return nil, fmt.Errorf("creating deployment record: %w", err)
	}

	// Store wasm artifact
	if err := s.artifactStore.Save(tenantID, appName, depID, bytes.NewReader(wasmBytes)); err != nil {
		return nil, fmt.Errorf("saving wasm artifact: %w", err)
	}

	return &domain.MigrationReport{
		Status:              domain.MigrationStatusSuccess,
		WasmStored:          true,
		DeploymentID:        &depID,
		AppName:             appName,
		PatternsTransformed:  patternsTransformed,
	}, nil
}

// detectTransformedPatterns scans WASI C output for known WASI function names
// and returns a list of PatternInfo describing what was transformed.
func detectTransformedPatterns(wasiC string) []domain.PatternInfo {
	transforms := []struct {
		contains string
		pattern  string
		wasi     string
	}{
		{"wasi_socket_tcp_create", "socket(AF_INET, SOCK_STREAM, 0)", "wasi_socket_tcp_create"},
		{"wasi_socket_tcp_start_bind", "bind(fd, addr, len)", "wasi_socket_tcp_start_bind"},
		{"wasi_socket_tcp_start_listen", "listen(fd, backlog)", "wasi_socket_tcp_start_listen"},
		{"wasi_socket_tcp_accept", "accept(fd, ...)", "wasi_socket_tcp_accept"},
		{"wasi_socket_tcp_start_connect", "connect(fd, addr, len)", "wasi_socket_tcp_start_connect"},
		{"wasi_output_stream_write", "send(fd, buf, len, flags)", "wasi_output_stream_write"},
		{"wasi_input_stream_read", "recv(fd, buf, len, flags)", "wasi_input_stream_read"},
		{"wasi_filesystem_open", "fopen(path, mode)", "wasi_filesystem_open"},
		{"wasi_ip_name_lookup_resolve", "gethostbyname(name)", "wasi_ip_name_lookup_resolve"},
	}

	var patterns []domain.PatternInfo
	seen := make(map[string]bool)
	for _, t := range transforms {
		if strings.Contains(wasiC, t.contains) && !seen[t.pattern] {
			seen[t.pattern] = true
			patterns = append(patterns, domain.PatternInfo{
				Line:             0,
				Pattern:          t.pattern,
				Snippet:          t.pattern,
				WasiEquivalent:    t.wasi,
				Transformability: "Auto-transformable",
			})
		}
	}
	return patterns
}

// validateWasm checks whether b is a valid wasm binary (magic number check).
func validateWasm(b []byte) bool {
	return bytes.HasPrefix(b, []byte{0x00, 0x61, 0x73, 0x6d})
}

// MigrateTree analyzes + transforms every C file in `entries` together
// and compiles them into a single wasm binary. M2.C9.
//
// Per file, two subprocesses are run:
//  1. `edge-migrate --transform <path>` — produces WASI C
//  2. `edge-migrate --analyze --json <path>` — produces a structured
//     `MigrationReport` JSON used to populate
//     `FileReport.patterns_detected` / `transformations` / `manual_review`
//     and `preprocessor`.
//
// If `--analyze --json` fails (older edge-migrate binary), the
// service falls back to the existing `detectTransformedPatterns`
// heuristic on the WASI C output. A `// TODO` below flags the
// removal point once edge-migrate ≥ v0.3 ships everywhere.
//
// All transformed C files are then compiled together in a single
// clang invocation (`--target=wasm32-wasip2 -nostdlib -I <tmpdir>`),
// producing one wasm binary. The wasm size is checked against
// `MaxArtifactSize` and the artifact + deployment row are written
// only on success.
//
// Per-file errors (parse failure, transform failure) don't abort the
// rest of the tree — the file gets a `FileReport` with
// `status: Failed` and processing continues.
//
// `entries` paths must be forward-slash-relative to the tree root
// (the handler validates). The service enforces the same
// `IsValidDeploymentAppName` regex as a defense-in-depth check.
func (s *MigrationService) MigrateTree(
	ctx context.Context,
	tenantID, appName, language string,
	entries []domain.FileEntry,
) (*domain.TreeMigrationReport, error) {
	// Defensive: handler also validates, but reject early here.
	if !IsValidDeploymentAppName(appName) {
		return nil, fmt.Errorf("invalid app name: %q", appName)
	}
	// M2 only supports C; the handler rejects other languages
	// before calling, so this is belt-and-suspenders.
	if language != "" && language != "c" {
		return nil, fmt.Errorf("unsupported language: %q (only \"c\" is supported in M2)", language)
	}
	if len(entries) == 0 {
		return nil, fmt.Errorf("no files in tree")
	}

	// Create a temp dir for the source files + transformed output.
	tmpDir, err := os.MkdirTemp("", "migrate-tree-*.d")
	if err != nil {
		return nil, fmt.Errorf("creating temp dir: %w", err)
	}
	defer os.RemoveAll(tmpDir)

	// Write each entry to <tmpDir>/<path>. Reject path traversal
	// (defense-in-depth; handler also validates).
	type writtenFile struct {
		path        string
		absPath     string
		wasiCPath   string // populated after transform
		report      domain.FileReport
		transformOK bool
	}
	written := make([]writtenFile, 0, len(entries))

	for _, e := range entries {
		clean := filepath.Clean(e.Path)
		if clean == "." || clean == ".." ||
			strings.HasPrefix(clean, "/") ||
			strings.HasPrefix(clean, "..") ||
			strings.Contains(clean, "\\") {
			return nil, fmt.Errorf("invalid file path: %q", e.Path)
		}
		abs := filepath.Join(tmpDir, clean)
		if err := os.MkdirAll(filepath.Dir(abs), 0o755); err != nil {
			return nil, fmt.Errorf("creating dir for %q: %w", e.Path, err)
		}
		if err := os.WriteFile(abs, []byte(e.Source), 0o644); err != nil {
			return nil, fmt.Errorf("writing %q: %w", e.Path, err)
		}
		written = append(written, writtenFile{path: e.Path, absPath: abs})
	}

	// Per-file subprocess: transform + analyze-json.
	// Continues on per-file failure; failures are captured into
	// FileReport.errors and the file's status is set to Failed.
	for i := range written {
		wf := &written[i]

		// 1) `edge-migrate --transform <path>` → WASI C output.
		// We write WASI C to <path>.wasi.c in the same dir so the
		// final clang invocation can pick them all up.
		edgeMigCmd := exec.CommandContext(ctx, s.edgeMigratePath, "--transform", wf.absPath)
		var edgeMigOut bytes.Buffer
		edgeMigCmd.Stdout = &edgeMigOut
		var edgeMigErr bytes.Buffer
		edgeMigCmd.Stderr = &edgeMigErr
		if err := edgeMigCmd.Run(); err != nil {
			wf.report = domain.FileReport{
				Path:   wf.path,
				Status: domain.MigrationStatusFailed,
				Errors: []domain.ErrorInfo{{
					Line:    0,
					Message: fmt.Sprintf("edge-migrate failed: %s — %s", err, edgeMigErr.String()),
				}},
			}
			continue
		}
		wasiC := edgeMigOut.String()
		wasiCPath := wf.absPath + ".wasi.c"
		if err := os.WriteFile(wasiCPath, []byte(wasiC), 0o644); err != nil {
			wf.report = domain.FileReport{
				Path:   wf.path,
				Status: domain.MigrationStatusFailed,
				Errors: []domain.ErrorInfo{{
					Line:    0,
					Message: fmt.Sprintf("writing wasi.c: %s", err),
				}},
			}
			continue
		}
		wf.wasiCPath = wasiCPath

		// 2) `edge-migrate --analyze --json <path>` → structured report.
		// On failure (older binary), fall back to detectTransformedPatterns.
		analyzeCmd := exec.CommandContext(ctx, s.edgeMigratePath, "--analyze-json", wf.absPath)
		var analyzeOut bytes.Buffer
		analyzeCmd.Stdout = &analyzeOut
		var analyzeErr bytes.Buffer
		analyzeCmd.Stderr = &analyzeErr
		var single domain.MigrationReport
		analyzeOK := false
		if err := analyzeCmd.Run(); err == nil {
			if jerr := json.Unmarshal(analyzeOut.Bytes(), &single); jerr == nil {
				analyzeOK = true
			}
		}
		// TODO: remove this fallback once edge-migrate ≥ v0.3 ships
		// everywhere (M2 follow-up #2). detectTransformedPatterns is
		// a heuristic — it can't tell manual-review from auto.
		if !analyzeOK {
			patterns := detectTransformedPatterns(wasiC)
			single = domain.MigrationReport{
				Status:              classifyFromPatterns(patterns),
				WasmStored:          false,
				AppName:             appName,
				PatternsDetected:    patterns,
				PatternsTransformed: patterns,
				PatternsManualReview: nil,
				Errors:              nil,
			}
		}
		// Promote the single-file MigrationReport into a per-file FileReport.
		fr := domain.FileReport{
			Path:             wf.path,
			Status:           single.Status,
			PatternsDetected: single.PatternsDetected,
			Transformations:  single.PatternsTransformed,
			ManualReview:     single.PatternsManualReview,
			Errors:           single.Errors,
			Preprocessor:     single.Preprocessor,
		}
		wf.report = fr
		wf.transformOK = true
	}

	// Build the per-file reports (in input order) and compute tree status.
	files := make([]domain.FileReport, 0, len(written))
	for _, wf := range written {
		files = append(files, wf.report)
	}

	// Compute tree-level aggregates inline (matches the Rust
	// TreeMigrationReport::from_files rules).
	status := aggregateTreeStatus(files)
	filesTotal := len(files)
	filesTransformed := 0
	filesManualReview := 0
	for _, f := range files {
		if len(f.Transformations) > 0 {
			filesTransformed++
		}
		if len(f.ManualReview) > 0 {
			filesManualReview++
		}
	}

	// If any file failed transformation, we cannot compile a complete
	// wasm — return the per-file report as a tree-level failure.
	// Skip clang; just report the partial state.
	anyTransformFailed := false
	for _, wf := range written {
		if !wf.transformOK {
			anyTransformFailed = true
			break
		}
	}
	if anyTransformFailed {
		return &domain.TreeMigrationReport{
			Status:            status,
			WasmStored:        false,
			AppName:           appName,
			Files:             files,
			FilesTotal:        filesTotal,
			FilesTransformed:  filesTransformed,
			FilesManualReview: filesManualReview,
			Errors: []domain.ErrorInfo{{
				Line:    0,
				Message: "one or more files failed to transform; no wasm built",
			}},
		}, nil
	}

	// Compile all transformed .wasi.c files in a single clang invocation.
	tmpWasm, err := os.CreateTemp("", "migrate-tree-*.wasm")
	if err != nil {
		return nil, fmt.Errorf("creating temp wasm: %w", err)
	}
	tmpWasmPath := tmpWasm.Name()
	tmpWasm.Close()
	defer os.Remove(tmpWasmPath)

	clangBin := filepath.Join(s.wasiSdkPath, "clang")
	args := []string{
		"--target=wasm32-wasip2", "-nostdlib",
		"-I", tmpDir,
		"-o", tmpWasmPath,
	}
	for _, wf := range written {
		args = append(args, wf.wasiCPath)
	}
	clangCmd := exec.CommandContext(ctx, clangBin, args...)
	var clangErrBuf bytes.Buffer
	clangCmd.Stderr = &clangErrBuf
	if err := clangCmd.Run(); err != nil {
		return &domain.TreeMigrationReport{
			Status:            status,
			WasmStored:        false,
			AppName:           appName,
			Files:             files,
			FilesTotal:        filesTotal,
			FilesTransformed:  filesTransformed,
			FilesManualReview: filesManualReview,
			Errors: []domain.ErrorInfo{{
				Line:    0,
				Message: fmt.Sprintf("clang failed: %s — %s", err, clangErrBuf.String()),
			}},
		}, nil
	}

	// Read + size-check the wasm.
	wasmBytes, err := os.ReadFile(tmpWasmPath)
	if err != nil {
		return nil, fmt.Errorf("reading compiled wasm: %w", err)
	}
	if int64(len(wasmBytes)) > MaxArtifactSize {
		return &domain.TreeMigrationReport{
			Status:            status,
			WasmStored:        false,
			AppName:           appName,
			Files:             files,
			FilesTotal:        filesTotal,
			FilesTransformed:  filesTransformed,
			FilesManualReview: filesManualReview,
			Errors: []domain.ErrorInfo{{
				Line:    0,
				Message: fmt.Sprintf("wasm exceeds %d bytes (MaxArtifactSize)", MaxArtifactSize),
			}},
		}, nil
	}
	if !validateWasm(wasmBytes) {
		return &domain.TreeMigrationReport{
			Status:            status,
			WasmStored:        false,
			AppName:           appName,
			Files:             files,
			FilesTotal:        filesTotal,
			FilesTransformed:  filesTransformed,
			FilesManualReview: filesManualReview,
			Errors: []domain.ErrorInfo{{
				Line:    0,
				Message: "compiled artifact failed wasm magic-number check",
			}},
		}, nil
	}

	// Persist: deployment row + artifact blob.
	depID := "d_" + uuid.New().String()
	hash := sha256.Sum256(wasmBytes)
	deployment := &domain.Deployment{
		ID:        depID,
		TenantID:  tenantID,
		AppName:   appName,
		Status:    "migrated",
		Hash:      hex.EncodeToString(hash[:]),
		CreatedAt: time.Now(),
	}
	if err := s.deploymentRepo.Create(ctx, deployment); err != nil {
		return nil, fmt.Errorf("creating deployment: %w", err)
	}
	if err := s.artifactStore.Save(tenantID, appName, depID, bytes.NewReader(wasmBytes)); err != nil {
		return nil, fmt.Errorf("saving artifact: %w", err)
	}

	return &domain.TreeMigrationReport{
		Status:            status,
		WasmStored:        true,
		DeploymentID:      &depID,
		AppName:           appName,
		Files:             files,
		FilesTotal:        filesTotal,
		FilesTransformed:  filesTransformed,
		FilesManualReview: filesManualReview,
	}, nil
}

// classifyFromPatterns maps a list of detected patterns to a
// MigrationStatus. Mirrors the Rust MigrationReport::from_pattern_matches
// rule: empty manual_review → Success; only manual_review → Failed;
// mixed → Partial. Used by the detectTransformedPatterns fallback
// path when --analyze --json is unavailable.
func classifyFromPatterns(patterns []domain.PatternInfo) domain.MigrationStatus {
	hasTransformed := false
	hasManual := false
	for _, p := range patterns {
		if p.Transformability == "NotTransformable" || p.Transformability == "Not-transformable" {
			hasManual = true
		} else {
			hasTransformed = true
		}
	}
	switch {
	case !hasManual:
		return domain.MigrationStatusSuccess
	case !hasTransformed:
		return domain.MigrationStatusFailed
	default:
		return domain.MigrationStatusPartial
	}
}

// aggregateTreeStatus mirrors the Rust TreeMigrationReport::from_files
// rules: any Failed → Failed; any Partial → Partial; else Success.
func aggregateTreeStatus(files []domain.FileReport) domain.MigrationStatus {
	if len(files) == 0 {
		return domain.MigrationStatusSuccess
	}
	anyFailed := false
	anyPartial := false
	for _, f := range files {
		switch f.Status {
		case domain.MigrationStatusFailed:
			anyFailed = true
		case domain.MigrationStatusPartial:
			anyPartial = true
		case domain.MigrationStatusSuccess:
		}
	}
	switch {
	case anyFailed:
		return domain.MigrationStatusFailed
	case anyPartial:
		return domain.MigrationStatusPartial
	default:
		return domain.MigrationStatusSuccess
	}
}
