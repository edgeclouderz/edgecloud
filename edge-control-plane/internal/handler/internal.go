package handler

import (
	"net/http"

	"github.com/edgeclouderz/edge-cloud/edge-control-plane/internal/service"
)

// InternalHandler handles internal worker-facing endpoints.
type InternalHandler struct {
	deploymentSvc *service.DeploymentService
}

func NewInternalHandler(deploymentSvc *service.DeploymentService) *InternalHandler {
	return &InternalHandler{deploymentSvc: deploymentSvc}
}

// Download serves Wasm artifacts to workers.
func (h *InternalHandler) Download(w http.ResponseWriter, r *http.Request) {
	deploymentID := r.PathValue("deploymentID")

	// TODO: Validate JWT from worker

	// Get deployment to find tenant and app
	deployment, err := h.deploymentSvc.GetDeployment(r.Context(), deploymentID)
	if err != nil || deployment == nil {
		http.Error(w, "not found", http.StatusNotFound)
		return
	}

	artifact, err := h.deploymentSvc.GetArtifact(r.Context(), deployment.TenantID, deployment.AppName, deploymentID)
	if err != nil {
		http.Error(w, "artifact not found", http.StatusNotFound)
		return
	}
	defer artifact.Close()

	w.Header().Set("Content-Type", "application/octet-stream")
	w.WriteHeader(http.StatusOK)
	// Copy artifact to response
	// buffer size 32KB
	buf := make([]byte, 32768)
	for {
		n, err := artifact.Read(buf)
		if n > 0 {
			if _, werr := w.Write(buf[:n]); werr != nil {
				return
			}
		}
		if err != nil {
			break
		}
	}
}
