package handler

import (
	"encoding/json"
	"errors"
	"io"
	"log"
	"net/http"

	"github.com/edgeclouderz/edge-cloud/edge-control-plane/internal/domain"
	"github.com/edgeclouderz/edge-cloud/edge-control-plane/internal/middleware"
	"github.com/edgeclouderz/edge-cloud/edge-control-plane/internal/service"
)

// InternalHandler handles internal worker-facing endpoints.
type InternalHandler struct {
	deploymentSvc *service.DeploymentService
	workerSvc     *service.WorkerService
}

func NewInternalHandler(deploymentSvc *service.DeploymentService, workerSvc *service.WorkerService) *InternalHandler {
	return &InternalHandler{
		deploymentSvc: deploymentSvc,
		workerSvc:     workerSvc,
	}
}

// Download serves Wasm artifacts to authenticated workers.
// Requires a valid worker JWT via Authorization: Bearer <token> header or ?jwt= query param.
func (h *InternalHandler) Download(w http.ResponseWriter, r *http.Request) {
	tenantID := middleware.GetWorkerTenantID(r.Context())
	deploymentID := r.PathValue("deploymentID")

	deployment, err := h.deploymentSvc.GetDeployment(r.Context(), tenantID, deploymentID)
	if err != nil || deployment == nil {
		http.Error(w, "not found", http.StatusNotFound)
		return
	}

	artifact, err := h.deploymentSvc.GetArtifact(r.Context(), deployment.TenantID, deployment.AppName, deployment.ID)
	if err != nil {
		http.Error(w, "artifact not found", http.StatusNotFound)
		return
	}
	defer artifact.Close()

	w.Header().Set("Content-Type", "application/octet-stream")
	w.WriteHeader(http.StatusOK)
	if _, err := io.Copy(w, artifact); err != nil {
		// client disconnected, nothing we can do
		return
	}
}

// RegisterWorker handles POST /api/internal/workers — worker registration.
func (h *InternalHandler) RegisterWorker(w http.ResponseWriter, r *http.Request) {
	tenantID := middleware.GetWorkerTenantID(r.Context())
	var req domain.RegisterWorkerRequest
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		http.Error(w, "invalid request body", http.StatusBadRequest)
		return
	}
	// Validate required fields.
	if req.WorkerID == "" || req.Region == "" {
		http.Error(w, "worker_id and region are required", http.StatusBadRequest)
		return
	}
	if err := h.workerSvc.Register(r.Context(), tenantID, &req); err != nil {
		switch {
		case errors.Is(err, service.ErrInvalidWorkerID):
			http.Error(w, `{"error": "invalid worker ID"}`, http.StatusBadRequest)
		case errors.Is(err, service.ErrRegionMismatch):
			http.Error(w, `{"error": "region mismatch"}`, http.StatusBadRequest)
		case errors.Is(err, service.ErrQuotaExceeded):
			http.Error(w, `{"error": "quota exceeded"}`, http.StatusTooManyRequests)
		default:
			log.Printf("internal error: %v", err)
			http.Error(w, `{"error": "internal error"}`, http.StatusInternalServerError)
		}
		return
	}
	w.WriteHeader(http.StatusCreated)
}

// ListWorkers handles GET /api/internal/workers — list workers for the authenticated tenant.
func (h *InternalHandler) ListWorkers(w http.ResponseWriter, r *http.Request) {
	tenantID := middleware.GetWorkerTenantID(r.Context())
	workers, err := h.workerSvc.ListByTenant(r.Context(), tenantID)
	if err != nil {
		http.Error(w, "failed to list workers", http.StatusInternalServerError)
		return
	}
	resp := map[string]interface{}{"workers": workers}
	w.Header().Set("Content-Type", "application/json")
	json.NewEncoder(w).Encode(resp)
}

// AppIngress handles GET /api/internal/apps/{appName}/ingress — diagnostic
// endpoint that returns whether an app is currently routable on a worker
// (and on which addr/port). Used by the CLI's `edge status` to validate a
// `live_url` is actually live. Restricted to the calling worker's tenant.
func (h *InternalHandler) AppIngress(w http.ResponseWriter, r *http.Request) {
	tenantID := middleware.GetWorkerTenantID(r.Context())
	appName := r.PathValue("appName")

	targets, err := h.workerSvc.ListRunningAppTargets(r.Context())
	if err != nil {
		log.Printf("internal error: %v", err)
		http.Error(w, `{"error": "internal error"}`, http.StatusInternalServerError)
		return
	}
	for _, t := range targets {
		if t.AppName == appName && t.TenantID == tenantID {
			w.Header().Set("Content-Type", "application/json")
			json.NewEncoder(w).Encode(map[string]interface{}{
				"ready":       true,
				"app_name":    t.AppName,
				"tenant_id":   t.TenantID,
				"worker_id":   t.WorkerID,
				"region":      t.Region,
				"worker_addr": t.WorkerAddr,
				"port":        t.Port,
			})
			return
		}
	}
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(http.StatusNotFound)
	json.NewEncoder(w).Encode(map[string]interface{}{
		"ready":    false,
		"app_name": appName,
		"reason":   "no running app found for this tenant",
	})
}
