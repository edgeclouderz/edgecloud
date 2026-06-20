package handler

import (
	"archive/zip"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"log"
	"mime/multipart"
	"net/http"
	"path/filepath"
	"strings"

	"github.com/edgeclouderz/edge-cloud/edge-control-plane/internal/domain"
	"github.com/edgeclouderz/edge-cloud/edge-control-plane/internal/middleware"
	"github.com/edgeclouderz/edge-cloud/edge-control-plane/internal/service"
)

// Tree upload limits.
const (
	// maxTreeBodyBytes is the hard cap on the request body for
	// POST /api/migrate-tree. Larger bodies are rejected mid-stream
	// by http.MaxBytesReader.
	maxTreeBodyBytes int64 = 50 << 20 // 50 MiB
	// maxTreeFiles is the cap on the number of files in a single tree
	// upload. Larger trees are rejected with 400.
	maxTreeFiles = 256
)

// treeUploadExts is the set of file extensions accepted in a tree
// upload. C: `.c`/`.h` (M2). Rust: `.rs` (M3). Other extensions are
// silently skipped — neither accepted nor rejected — so a tarball
// with a `Makefile` or `Cargo.toml` still works.
var treeUploadExts = map[string]bool{
	".c":  true,
	".h":  true,
	".rs": true,
}

// isClientMigrationError reports whether `err` from the migration
// service is a request-level failure (the request was syntactically
// valid but the source didn't transform / compile / fit). The
// handler maps these to HTTP 422 and emits the structured report
// body so the caller can read the per-pattern error detail. All
// other errors are server-level (DB outage, IO, etc.) and map to 500.
func isClientMigrationError(err error) bool {
	return errors.Is(err, service.ErrMigrateTreeFailed) ||
		errors.Is(err, service.ErrMigrationFailed) ||
		errors.Is(err, service.ErrEdgeMigrateFailed) ||
		errors.Is(err, service.ErrClangFailed) ||
		errors.Is(err, service.ErrRustcFailed)
}

// MigrationHandler handles migration requests.
type MigrationHandler struct {
	migrationSvc *service.MigrationService
}

// NewMigrationHandler creates a MigrationHandler.
func NewMigrationHandler(migrationSvc *service.MigrationService) *MigrationHandler {
	return &MigrationHandler{migrationSvc: migrationSvc}
}

// Migrate handles POST /api/migrate — accepts a C source file, transforms it,
// and returns a MigrationReport.
func (h *MigrationHandler) Migrate(w http.ResponseWriter, r *http.Request) {
	tenantID := middleware.GetTenantID(r.Context())
	if tenantID == "" {
		http.Error(w, `{"error":"missing tenant ID"}`, http.StatusUnauthorized)
		return
	}

	if err := r.ParseMultipartForm(50 << 20); err != nil {
		http.Error(w, `{"error":"failed to parse multipart form"}`, http.StatusBadRequest)
		return
	}

	filename := r.MultipartForm.Value["filename"]
	if len(filename) == 0 || filename[0] == "" {
		http.Error(w, `{"error":"missing filename field"}`, http.StatusBadRequest)
		return
	}
	// Reject path-traversal early — derived app_name is what actually gets
	// written to the DB and used in the registry path. The service has a
	// defense-in-depth check; this one gives a clear 400 to the client.
	if containsPathTraversal(strings.TrimSuffix(filename[0], ".c")) {
		http.Error(w, `{"error":"filename must not contain path-traversal characters"}`, http.StatusBadRequest)
		return
	}

	language := r.MultipartForm.Value["language"]
	if len(language) == 0 || (language[0] != "c" && language[0] != "rust") {
		http.Error(w, `{"error":"only c and rust are supported"}`, http.StatusBadRequest)
		return
	}

	fileParts := r.MultipartForm.File["file"]
	if len(fileParts) == 0 {
		http.Error(w, `{"error":"missing file field"}`, http.StatusBadRequest)
		return
	}

	srcFile, err := fileParts[0].Open()
	if err != nil {
		http.Error(w, `{"error": "failed to open file"}`, http.StatusInternalServerError)
		return
	}
	defer srcFile.Close()

	source, err := io.ReadAll(srcFile)
	if err != nil {
		http.Error(w, `{"error": "failed to read file"}`, http.StatusInternalServerError)
		return
	}

	report, err := h.migrationSvc.Migrate(r.Context(), tenantID, filename[0], language[0], string(source))
	if err != nil {
		if isClientMigrationError(err) {
			// Source-level failure (didn't transform, didn't compile,
			// etc.). Surface as 422 with the structured report body
			// so the caller can read the per-pattern error detail.
			w.Header().Set("Content-Type", "application/json")
			w.WriteHeader(http.StatusUnprocessableEntity)
			if encErr := json.NewEncoder(w).Encode(report); encErr != nil {
				log.Printf("Migrate encode error after 422: %v", encErr)
			}
			return
		}
		log.Printf("Migrate internal error: %v", err)
		http.Error(w, `{"error":"internal error"}`, http.StatusInternalServerError)
		return
	}

	w.Header().Set("Content-Type", "application/json")
	if err := json.NewEncoder(w).Encode(report); err != nil {
		http.Error(w, `{"error": "internal error"}`, http.StatusInternalServerError)
	}
}

// MigrateTree handles POST /api/migrate-tree. Accepts a multi-file
// source tree (`language: c` or `language: rust`) in two wire formats:
//
//   - Variant A — multipart parts: one `file` per source file, plus
//     a `tree` JSON manifest `{"files": [...]}`. Each `file` part's
//     filename must match an entry in the manifest.
//
//   - Variant B — zip archive: a single `tree` part with
//     `Content-Type: application/zip`. Zip entries are the source of
//     truth; the `tree` JSON manifest is not used.
//
// Both variants produce a `domain.TreeMigrationReport` JSON response.
// See `edge-migrate/docs/design.md` §6.1.2.
func (h *MigrationHandler) MigrateTree(w http.ResponseWriter, r *http.Request) {
	tenantID := middleware.GetTenantID(r.Context())
	if tenantID == "" {
		http.Error(w, `{"error":"missing tenant ID"}`, http.StatusUnauthorized)
		return
	}

	// Cap the request body up front so a malicious caller can't pin
	// a 10 GB upload mid-stream.
	r.Body = http.MaxBytesReader(w, r.Body, maxTreeBodyBytes)

	if err := r.ParseMultipartForm(maxTreeBodyBytes); err != nil {
		// MaxBytesReader returns *http.MaxBytesError once the cap is
		// hit. Detect via errors.As so we don't string-match the
		// error message.
		var maxErr *http.MaxBytesError
		if errors.As(err, &maxErr) {
			http.Error(w, `{"error":"request body too large"}`, http.StatusRequestEntityTooLarge)
			return
		}
		http.Error(w, `{"error":"failed to parse multipart form"}`, http.StatusBadRequest)
		return
	}

	// app_name is required for both variants.
	appName := r.MultipartForm.Value["app_name"]
	if len(appName) == 0 || appName[0] == "" {
		http.Error(w, `{"error":"missing app_name field"}`, http.StatusBadRequest)
		return
	}
	if !service.IsValidDeploymentAppName(appName[0]) {
		http.Error(w, `{"error":"invalid app_name"}`, http.StatusBadRequest)
		return
	}

	// Language gate. M2 accepted only C; M3 widens to c + rust.
	language := r.MultipartForm.Value["language"]
	if len(language) == 0 || (language[0] != "c" && language[0] != "rust") {
		http.Error(w, `{"error":"only c and rust are supported"}`, http.StatusBadRequest)
		return
	}

	// Detect variant: if `tree` is a file part, it's variant B (zip).
	// Otherwise it's variant A (multipart parts + JSON manifest).
	treeFiles := r.MultipartForm.File["tree"]
	var entries []domain.FileEntry
	if len(treeFiles) > 0 {
		// Variant B: zip.
		e, err := readZipEntries(treeFiles[0], maxTreeFiles, maxTreeBodyBytes)
		if err != nil {
			http.Error(w, fmt.Sprintf(`{"error":%q}`, err.Error()), http.StatusBadRequest)
			return
		}
		entries = e
	} else {
		// Variant A: multipart parts + JSON manifest.
		treeManifest := r.MultipartForm.Value["tree"]
		if len(treeManifest) == 0 {
			http.Error(w, `{"error":"missing tree manifest or zip"}`, http.StatusBadRequest)
			return
		}
		var manifest struct {
			Files []string `json:"files"`
		}
		if err := json.Unmarshal([]byte(treeManifest[0]), &manifest); err != nil {
			http.Error(w, `{"error":"invalid tree manifest JSON"}`, http.StatusBadRequest)
			return
		}
		fileParts := r.MultipartForm.File["file"]
		if len(fileParts) == 0 {
			http.Error(w, `{"error":"missing file parts"}`, http.StatusBadRequest)
			return
		}
		if len(manifest.Files) != len(fileParts) {
			http.Error(w, `{"error":"manifest mismatch: file count differs"}`, http.StatusBadRequest)
			return
		}
		if len(manifest.Files) > maxTreeFiles {
			http.Error(w, fmt.Sprintf(`{"error":"too many files: max %d"}`, maxTreeFiles), http.StatusBadRequest)
			return
		}
		// Build path→part map, reject duplicates and bad paths.
		partByName := make(map[string]*multipart.FileHeader, len(fileParts))
		for _, fp := range fileParts {
			if !isSafeFilePath(fp.Filename) {
				http.Error(w, fmt.Sprintf(`{"error":"unsafe file path: %q"}`, fp.Filename), http.StatusBadRequest)
				return
			}
			partByName[normalizeFileName(fp.Filename)] = fp
		}
		for _, p := range manifest.Files {
			if !isSafeFilePath(p) {
				http.Error(w, fmt.Sprintf(`{"error":"unsafe manifest path: %q"}`, p), http.StatusBadRequest)
				return
			}
			part := partByName[normalizeFileName(p)]
			if part == nil {
				http.Error(w, fmt.Sprintf(`{"error":"manifest mismatch: missing file for %q"}`, p), http.StatusBadRequest)
				return
			}
			src, err := part.Open()
			if err != nil {
				http.Error(w, `{"error":"failed to open file part"}`, http.StatusInternalServerError)
				return
			}
			body, err := io.ReadAll(src)
			src.Close()
			if err != nil {
				http.Error(w, `{"error":"failed to read file part"}`, http.StatusInternalServerError)
				return
			}
			entries = append(entries, domain.FileEntry{Path: p, Source: string(body)})
		}
	}

	report, err := h.migrationSvc.MigrateTree(r.Context(), tenantID, appName[0], language[0], entries)
	if err != nil {
		if isClientMigrationError(err) {
			// Source-level failure (a file didn't transform, the
			// final compile failed, the artifact is oversized, etc.).
			// Surface as 422 with the structured report body so the
			// caller can read the per-file / tree-level error detail.
			w.Header().Set("Content-Type", "application/json")
			w.WriteHeader(http.StatusUnprocessableEntity)
			if encErr := json.NewEncoder(w).Encode(report); encErr != nil {
				log.Printf("MigrateTree encode error after 422: %v", encErr)
			}
			return
		}
		log.Printf("MigrateTree internal error: %v", err)
		http.Error(w, `{"error":"internal error"}`, http.StatusInternalServerError)
		return
	}

	w.Header().Set("Content-Type", "application/json")
	if err := json.NewEncoder(w).Encode(report); err != nil {
		log.Printf("MigrateTree encode error: %v", err)
		http.Error(w, `{"error":"internal error"}`, http.StatusInternalServerError)
	}
}

// multipartFilePart is intentionally a thin alias of
// *multipart.FileHeader. Kept as a named type so the call sites read
// naturally and the handler tests can substitute a stub.
type multipartFilePart = multipart.FileHeader

// readZipEntries opens the uploaded zip, validates each entry name
// (zip-slip protection), and returns the supported source files as
// FileEntry slices. The accepted extensions live in `treeUploadExts`
// (C: `.c`/`.h`, Rust: `.rs`). Caps the number of files and total
// decompressed size.
func readZipEntries(header *multipart.FileHeader, maxFiles int, maxBody int64) ([]domain.FileEntry, error) {
	src, err := header.Open()
	if err != nil {
		return nil, fmt.Errorf("opening zip part: %w", err)
	}
	defer src.Close()

	zr, err := zip.NewReader(src, header.Size)
	if err != nil {
		return nil, fmt.Errorf("reading zip: %w", err)
	}
	var entries []domain.FileEntry
	var total int64
	for _, f := range zr.File {
		if f.FileInfo().IsDir() {
			continue
		}
		// Reject any name that's not a relative, safe source-tree path.
		name := strings.ReplaceAll(f.Name, "\\", "/")
		if !isSafeFilePath(name) {
			return nil, fmt.Errorf("unsafe zip entry: %q", f.Name)
		}
		ext := strings.ToLower(filepath.Ext(name))
		if !treeUploadExts[ext] {
			continue
		}
		if len(entries) >= maxFiles {
			return nil, fmt.Errorf("too many files: max %d", maxFiles)
		}
		rc, err := f.Open()
		if err != nil {
			return nil, fmt.Errorf("opening zip entry %q: %w", f.Name, err)
		}
		body, err := io.ReadAll(io.LimitReader(rc, maxBody+1))
		rc.Close()
		if err != nil {
			return nil, fmt.Errorf("reading zip entry %q: %w", f.Name, err)
		}
		total += int64(len(body))
		if total > maxBody {
			return nil, fmt.Errorf("decompressed zip too large")
		}
		entries = append(entries, domain.FileEntry{Path: name, Source: string(body)})
	}
	return entries, nil
}

// isSafeFilePath rejects absolute paths, parent-directory escapes,
// backslashes, and Windows drive letters — used by both the
// multipart manifest path validator and the zip entry name validator.
func isSafeFilePath(p string) bool {
	if p == "" {
		return false
	}
	if strings.HasPrefix(p, "/") || strings.HasPrefix(p, "\\") {
		return false
	}
	if strings.Contains(p, "\\") {
		return false
	}
	clean := filepath.ToSlash(filepath.Clean(p))
	if clean == "." || clean == ".." || strings.HasPrefix(clean, "../") {
		return false
	}
	// Windows drive letter: "C:" or "C:foo"
	if len(clean) >= 2 && clean[1] == ':' {
		return false
	}
	return true
}

// normalizeFileName normalizes a multipart part filename by
// stripping the directory (e.g. "src/main.c" → "main.c"). The
// comparison then becomes name-based. The handler matches the full
// path on the manifest side; the part filename is just the basename
// per HTTP convention.
func normalizeFileName(s string) string {
	s = strings.ReplaceAll(s, "\\", "/")
	if i := strings.LastIndex(s, "/"); i >= 0 {
		return s[i+1:]
	}
	return s
}
