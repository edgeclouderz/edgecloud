package handler

import (
	"encoding/json"
	"io"
	"log"
	"net/http"
	"strings"

	"github.com/edgeclouderz/edge-cloud/edge-control-plane/internal/handler/httperror"
	"github.com/edgeclouderz/edge-cloud/edge-control-plane/internal/middleware"
	"github.com/edgeclouderz/edge-cloud/edge-control-plane/internal/service"
)

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
		httperror.Unauthorized(w, "missing tenant ID")
		return
	}

	if err := r.ParseMultipartForm(50 << 20); err != nil {
		httperror.BadRequest(w, "failed to parse multipart form")
		return
	}

	filename := r.MultipartForm.Value["filename"]
	if len(filename) == 0 || filename[0] == "" {
		httperror.BadRequest(w, "missing filename field")
		return
	}
	// Reject path-traversal early — derived app_name is what actually gets
	// written to the DB and used in the registry path. The service has a
	// defense-in-depth check; this one gives a clear 400 to the client.
	if containsPathTraversal(strings.TrimSuffix(filename[0], ".c")) {
		httperror.BadRequest(w, "filename must not contain path-traversal characters")
		return
	}

	language := r.MultipartForm.Value["language"]
	if len(language) == 0 || language[0] != "c" {
		httperror.BadRequest(w, "only C language is supported")
		return
	}

	fileParts := r.MultipartForm.File["file"]
	if len(fileParts) == 0 {
		httperror.BadRequest(w, "missing file field")
		return
	}

	srcFile, err := fileParts[0].Open()
	if err != nil {
		httperror.InternalError(w)
		return
	}
	defer srcFile.Close()

	source, err := io.ReadAll(srcFile)
	if err != nil {
		httperror.InternalError(w)
		return
	}

	report, err := h.migrationSvc.Migrate(r.Context(), tenantID, filename[0], language[0], string(source))
	if err != nil {
		log.Printf("internal error: %v", err)
		httperror.InternalError(w)
		return
	}

	w.Header().Set("Content-Type", "application/json")
	if err := json.NewEncoder(w).Encode(report); err != nil {
		httperror.InternalError(w)
	}
}
