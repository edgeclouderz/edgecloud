package handler

import (
	"context"
	"encoding/json"
	"log"
	"net/http"

	"github.com/edgeclouderz/edge-cloud/edge-control-plane/internal/domain"
	"github.com/edgeclouderz/edge-cloud/edge-control-plane/internal/handler/httperror"
	"github.com/edgeclouderz/edge-cloud/edge-control-plane/internal/middleware"
)

// TrafficHandler handles traffic split HTTP requests.
type TrafficHandler struct {
	trafficSvc TrafficServiceInterface
}

// TrafficServiceInterface is the subset of service.TrafficService needed by the handler.
type TrafficServiceInterface interface {
	SetTraffic(ctx context.Context, tenantID, appName string, entries []domain.TrafficSplitEntry) error
	GetTraffic(ctx context.Context, tenantID, appName string) ([]*domain.TrafficSplit, error)
}

// NewTrafficHandler creates a TrafficHandler.
func NewTrafficHandler(trafficSvc TrafficServiceInterface) *TrafficHandler {
	return &TrafficHandler{trafficSvc: trafficSvc}
}

// SetTraffic handles PUT /api/v1/apps/{appName}/traffic.
// Body: {"splits": [{"deployment_id": "d_v1", "weight": 95}, {"deployment_id": "d_v2", "weight": 5}]}
func (h *TrafficHandler) SetTraffic(w http.ResponseWriter, r *http.Request) {
	tenantID := middleware.GetTenantID(r.Context())
	appName := r.PathValue("appName")

	var req domain.TrafficSplitRequest
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		httperror.BadRequestCtx(w, r, "invalid JSON body")
		return
	}

	if err := h.trafficSvc.SetTraffic(r.Context(), tenantID, appName, req.Splits); err != nil {
		log.Printf("SetTraffic error: %v", err)
		httperror.InternalErrorCtx(w, r)
		return
	}

	w.Header().Set("Content-Type", "application/json")
	json.NewEncoder(w).Encode(map[string]string{"status": "traffic_set"})
}

// GetTraffic handles GET /api/v1/apps/{appName}/traffic.
func (h *TrafficHandler) GetTraffic(w http.ResponseWriter, r *http.Request) {
	tenantID := middleware.GetTenantID(r.Context())
	appName := r.PathValue("appName")

	splits, err := h.trafficSvc.GetTraffic(r.Context(), tenantID, appName)
	if err != nil {
		log.Printf("GetTraffic error: %v", err)
		httperror.InternalErrorCtx(w, r)
		return
	}

	w.Header().Set("Content-Type", "application/json")
	json.NewEncoder(w).Encode(map[string]interface{}{
		"app_name": appName,
		"splits":   splits,
	})
}

// GetTrafficInternal handles GET /api/v1/internal/traffic/{tenantID}/{appName}.
// Mounted under the `internalAuth` middleware (shared-secret header), this is
// the read endpoint the edge-ingress polls to apply Caddy weights. Unlike
// GetTraffic, the tenant is not derived from an authenticated context — it
// comes from the URL path because the ingress is a service-to-service caller,
// not a tenant. The split query is the same as GetTraffic's; only the
// authentication and how the tenant is identified differ.
func (h *TrafficHandler) GetTrafficInternal(w http.ResponseWriter, r *http.Request) {
	tenantID := r.PathValue("tenantID")
	appName := r.PathValue("appName")

	splits, err := h.trafficSvc.GetTraffic(r.Context(), tenantID, appName)
	if err != nil {
		log.Printf("GetTrafficInternal error: %v", err)
		httperror.InternalErrorCtx(w, r)
		return
	}

	w.Header().Set("Content-Type", "application/json")
	json.NewEncoder(w).Encode(map[string]interface{}{
		"app_name": appName,
		"splits":   splits,
	})
}
