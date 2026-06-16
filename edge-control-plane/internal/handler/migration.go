package handler

import (
	"encoding/json"
	"fmt"
	"io"
	"net/http"

	"github.com/edgeclouderz/edge-cloud/edge-control-plane/internal/middleware"
	"github.com/edgeclouderz/edge-cloud/edge-control-plane/internal/service"
)

type MigrationHandler struct {
	migrationSvc *service.MigrationService
}

func NewMigrationHandler(migrationSvc *service.MigrationService) *MigrationHandler {
	return &MigrationHandler{migrationSvc: migrationSvc}
}

func (h *MigrationHandler) Migrate(w http.ResponseWriter, r *http.Request) {
	tenantID := middleware.GetTenantID(r.Context())

	if err := r.ParseMultipartForm(50 << 20); err != nil {
		http.Error(w, `{"error": "failed to parse multipart form"}`, http.StatusBadRequest)
		return
	}

	filenameParts := r.MultipartForm.Value["filename"]
	languageParts := r.MultipartForm.Value["language"]
	if len(filenameParts) == 0 || len(languageParts) == 0 {
		http.Error(w, `{"error": "missing filename or language field"}`, http.StatusBadRequest)
		return
	}
	filename := filenameParts[0]
	language := languageParts[0]

	fileParts := r.MultipartForm.File["file"]
	if len(fileParts) == 0 {
		http.Error(w, `{"error": "missing file field"}`, http.StatusBadRequest)
		return
	}

	srcFile, err := fileParts[0].Open()
	if err != nil {
		http.Error(w, `{"error": "failed to open file"}`, http.StatusBadRequest)
		return
	}
	defer srcFile.Close()

	source, err := io.ReadAll(srcFile)
	if err != nil {
		http.Error(w, `{"error": "failed to read file"}`, http.StatusBadRequest)
		return
	}

	report, err := h.migrationSvc.Migrate(r.Context(), tenantID, filename, language, string(source))
	if err != nil {
		http.Error(w, fmt.Sprintf(`{"error": "%s"}`, err.Error()), http.StatusInternalServerError)
		return
	}

	w.Header().Set("Content-Type", "application/json")
	json.NewEncoder(w).Encode(report)
}
