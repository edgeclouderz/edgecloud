package handler

import (
	"encoding/json"
	"log"
	"net/http"

	"github.com/edgeclouderz/edge-cloud/edge-control-plane/internal/middleware"
	"github.com/edgeclouderz/edge-cloud/edge-control-plane/internal/service"
)

// EgressHandler exposes the tenant self-service egress allowlist API.
//
// GET  /api/egress  — return the current allowlist for the authenticated tenant
// PUT  /api/egress  — replace the allowlist; triggers republish of active deployments
type EgressHandler struct {
	tenantSvc     *service.TenantService
	deploymentSvc *service.DeploymentService
}

func NewEgressHandler(tenantSvc *service.TenantService, deploymentSvc *service.DeploymentService) *EgressHandler {
	return &EgressHandler{tenantSvc: tenantSvc, deploymentSvc: deploymentSvc}
}

type egressResponse struct {
	Allowlist []string `json:"allowlist"`
}

type updateEgressRequest struct {
	Allowlist []string `json:"allowlist"`
}

// Get returns the authenticated tenant's current outbound allowlist.
func (h *EgressHandler) Get(w http.ResponseWriter, r *http.Request) {
	tenantID := middleware.GetTenantID(r.Context())

	list, err := h.tenantSvc.GetEgressAllowlist(r.Context(), tenantID)
	if err != nil {
		http.Error(w, `{"error":"internal error"}`, http.StatusInternalServerError)
		return
	}

	w.Header().Set("Content-Type", "application/json")
	json.NewEncoder(w).Encode(egressResponse{Allowlist: list})
}

// Update replaces the authenticated tenant's outbound allowlist and immediately
// republishes TaskMessages for all active deployments so workers enforce the
// new policy without a manual re-activate.
func (h *EgressHandler) Update(w http.ResponseWriter, r *http.Request) {
	tenantID := middleware.GetTenantID(r.Context())

	var req updateEgressRequest
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		http.Error(w, `{"error":"invalid request body"}`, http.StatusBadRequest)
		return
	}
	// Treat a missing field the same as an explicit empty list (allow-all).
	if req.Allowlist == nil {
		req.Allowlist = []string{}
	}

	if err := h.tenantSvc.UpdateEgressAllowlist(r.Context(), tenantID, req.Allowlist); err != nil {
		errBody, _ := json.Marshal(map[string]string{"error": err.Error()})
		w.Header().Set("Content-Type", "application/json")
		w.WriteHeader(http.StatusBadRequest)
		w.Write(errBody)
		return
	}

	// Best-effort republish: workers will pick up the new allowlist on the
	// next TaskMessage; a blip here doesn't roll back the DB write.
	if err := h.deploymentSvc.RepublishActiveDeployments(r.Context(), tenantID); err != nil {
		log.Printf("egress update: republish failed for tenant %s: %v", tenantID, err)
	}

	w.Header().Set("Content-Type", "application/json")
	json.NewEncoder(w).Encode(egressResponse{Allowlist: req.Allowlist})
}
