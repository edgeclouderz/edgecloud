package service

import (
	"context"
	"encoding/json"
	"fmt"
	"time"

	"github.com/edgeclouderz/edge-cloud/edge-control-plane/internal/domain"
	"github.com/edgeclouderz/edge-cloud/edge-control-plane/internal/repository"
)

// ClusterView is the operator-facing snapshot of every region + worker
// in the platform. Returned by GET /api/admin/cluster.
type ClusterView struct {
	GeneratedAt time.Time             `json:"generated_at"`
	Regions     map[string]RegionView `json:"regions"`
}

// RegionView groups workers by region and reports the average app
// count per worker — useful to spot skew after app-pinning (#86).
type RegionView struct {
	Workers       []WorkerView `json:"workers"`
	AppsPerWorker int          `json:"apps_per_worker_avg"`
}

// WorkerView is the per-worker projection of the cluster view.
type WorkerView struct {
	WorkerID string    `json:"worker_id"`
	Region   string    `json:"region"`
	IP       string    `json:"ip,omitempty"`
	LastSeen time.Time `json:"last_seen"`
	AppCount int       `json:"app_count"`
	MemoryMB int       `json:"memory_mb"`
}

// ClusterServiceInterface allows handler tests to substitute a mock.
type ClusterServiceInterface interface {
	List(ctx context.Context) (*ClusterView, error)
}

// ClusterService builds the cluster view from the worker + worker_status
// repositories. Both queries are best-effort: a worker with no status
// row simply gets AppCount=0 (heartbeat hasn't arrived yet).
type ClusterService struct {
	workerRepo *repository.WorkerRepository
}

// NewClusterService constructs a ClusterService.
func NewClusterService(workerRepo *repository.WorkerRepository) *ClusterService {
	return &ClusterService{workerRepo: workerRepo}
}

// List returns the current cluster view.
func (s *ClusterService) List(ctx context.Context) (*ClusterView, error) {
	workers, err := s.workerRepo.List(ctx)
	if err != nil {
		return nil, fmt.Errorf("listing workers: %w", err)
	}

	view := &ClusterView{
		GeneratedAt: time.Now().UTC(),
		Regions:     make(map[string]RegionView),
	}

	// Bucket workers by region. Within each region we accumulate the
	// running-app count (from worker_status.apps, parsed as a JSON object)
	// and the per-worker AppCount for the view.
	regionAppTotals := make(map[string]int)
	regionWorkerCount := make(map[string]int)
	regionViews := make(map[string][]WorkerView)

	for _, w := range workers {
		appCount := 0
		status, err := s.workerRepo.GetStatus(ctx, w.ID)
		if err == nil && status != nil {
			var apps map[string]domain.AppStatus
			if jsonErr := json.Unmarshal(status.Apps, &apps); jsonErr == nil {
				appCount = len(apps)
			}
		}

		wv := WorkerView{
			WorkerID: w.ID,
			Region:   w.Region,
			LastSeen: w.LastSeen,
			AppCount: appCount,
			MemoryMB: w.MemoryMB,
		}
		if w.IP != nil {
			wv.IP = *w.IP
		}
		regionViews[w.Region] = append(regionViews[w.Region], wv)
		regionAppTotals[w.Region] += appCount
		regionWorkerCount[w.Region]++
	}

	for region, ws := range regionViews {
		total := regionAppTotals[region]
		count := regionWorkerCount[region]
		avg := 0
		if count > 0 {
			avg = total / count
		}
		view.Regions[region] = RegionView{
			Workers:       ws,
			AppsPerWorker: avg,
		}
	}
	return view, nil
}
