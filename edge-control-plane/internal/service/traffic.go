package service

import (
	"context"
	"fmt"
	"log"
	"strings"
	"time"

	"github.com/edgeclouderz/edge-cloud/edge-control-plane/internal/domain"
	"github.com/edgeclouderz/edge-cloud/edge-control-plane/internal/nats"
	"github.com/edgeclouderz/edge-cloud/edge-control-plane/internal/repository"
)

// TrafficService handles traffic split business logic.
type TrafficService struct {
	splitRepo      *repository.TrafficSplitRepository
	deploymentRepo *repository.DeploymentRepository
	activeRepo     *repository.ActiveDeploymentRepository
	appEnvRepo     *repository.AppEnvRepository
	tenantRepo     *repository.TenantRepository
	quotaRepo      *repository.QuotaRepository
	publisher      nats.Publisher
	// defaultRegion is the fallback when none of the splits' deployments
	// declare any regions of their own. Mirrors DeploymentService.defaultRegion
	// (which gets it from config) so a control plane that runs without an
	// explicit region still publishes to a subject every worker subscribes to.
	defaultRegion string
}

// NewTrafficService creates a TrafficService.
func NewTrafficService(
	splitRepo *repository.TrafficSplitRepository,
	deploymentRepo *repository.DeploymentRepository,
	activeRepo *repository.ActiveDeploymentRepository,
	appEnvRepo *repository.AppEnvRepository,
	tenantRepo *repository.TenantRepository,
	quotaRepo *repository.QuotaRepository,
	publisher nats.Publisher,
	defaultRegion string,
) *TrafficService {
	return &TrafficService{
		splitRepo:      splitRepo,
		deploymentRepo: deploymentRepo,
		activeRepo:     activeRepo,
		appEnvRepo:     appEnvRepo,
		tenantRepo:     tenantRepo,
		quotaRepo:      quotaRepo,
		publisher:      publisher,
		defaultRegion:  defaultRegion,
	}
}

// ValidateSum checks that the sum of weights equals 100.
func ValidateSum(splits []*domain.TrafficSplit) error {
	var total int
	for _, s := range splits {
		total += s.Weight
	}
	if total != 100 {
		return fmt.Errorf("weights must sum to 100, got %d", total)
	}
	return nil
}

// SetTraffic atomically sets the traffic splits for an app.
// Each deployment_id is validated to belong to the tenant and app.
// Sum of weights must equal 100.
func (s *TrafficService) SetTraffic(ctx context.Context, tenantID, appName string, entries []domain.TrafficSplitEntry) error {
	if len(entries) == 0 {
		// Clearing all splits is a valid operation — equivalent to no canary.
		return s.splitRepo.DeleteAllForApp(ctx, tenantID, appName)
	}

	splits := make([]*domain.TrafficSplit, len(entries))
	for i, e := range entries {
		d, err := s.deploymentRepo.GetByID(ctx, e.DeploymentID)
		if err != nil || d == nil {
			return fmt.Errorf("deployment %q not found", e.DeploymentID)
		}
		if d.TenantID != tenantID || d.AppName != appName {
			return fmt.Errorf("deployment %q not found", e.DeploymentID)
		}
		splits[i] = &domain.TrafficSplit{
			TenantID:     tenantID,
			AppName:      appName,
			DeploymentID: e.DeploymentID,
			Weight:       e.Weight,
		}
	}

	if err := ValidateSum(splits); err != nil {
		return err
	}

	if err := s.splitRepo.Set(ctx, splits); err != nil {
		return fmt.Errorf("setting traffic split: %w", err)
	}

	// Publish task update to activate all deployments in the split concurrently.
	return s.publishTaskUpdate(ctx, tenantID, appName)
}

// GetTraffic returns the current traffic splits for an app.
func (s *TrafficService) GetTraffic(ctx context.Context, tenantID, appName string) ([]*domain.TrafficSplit, error) {
	splits, err := s.splitRepo.Get(ctx, tenantID, appName)
	if err != nil {
		return nil, err
	}
	return splits, nil
}

// ClearTraffic removes all traffic splits for an app.
func (s *TrafficService) ClearTraffic(ctx context.Context, tenantID, appName string) error {
	return s.splitRepo.DeleteAllForApp(ctx, tenantID, appName)
}

// publishTaskUpdate sends a TaskMessage that tells workers to run all
// deployments in the traffic split concurrently.
func (s *TrafficService) publishTaskUpdate(ctx context.Context, tenantID, appName string) error {
	splits, err := s.splitRepo.Get(ctx, tenantID, appName)
	if err != nil {
		return fmt.Errorf("fetching splits: %w", err)
	}
	if len(splits) == 0 {
		return nil // nothing to publish
	}

	envs, err := s.appEnvRepo.List(ctx, tenantID, appName)
	if err != nil {
		return fmt.Errorf("listing env vars: %w", err)
	}
	envMap := make(map[string]string)
	for _, e := range envs {
		envMap[e.EnvKey] = e.EnvValue
	}

	tenant, err := s.tenantRepo.GetByID(ctx, tenantID)
	if err != nil || tenant == nil {
		return fmt.Errorf("tenant not found")
	}

	quota, err := s.quotaRepo.GetByTenantID(ctx, tenantID)
	if err != nil {
		return fmt.Errorf("getting quota: %w", err)
	}
	maxMemoryMB := 256
	if quota != nil && quota.MaxMemoryMB > 0 {
		maxMemoryMB = quota.MaxMemoryMB
	}

	// Build the routes list for the nats.AppConfig.
	// Each route carries its OWN deployment_hash — the worker needs the
	// per-route hash to download and verify the right artifact (the
	// top-level AppConfig.DeploymentHash only covers the primary). All
	// routes share the same env/allowlist/max_memory from the app config.
	var primaryHash string
	routes := make([]nats.DeploymentRoute, len(splits))
	for i, sp := range splits {
		d, err := s.deploymentRepo.GetByID(ctx, sp.DeploymentID)
		if err != nil || d == nil {
			return fmt.Errorf("deployment %q not found", sp.DeploymentID)
		}
		routes[i] = nats.DeploymentRoute{
			DeploymentID:   sp.DeploymentID,
			DeploymentHash: d.Hash,
			Weight:         sp.Weight,
		}
		if i == 0 {
			primaryHash = d.Hash
		}
	}

	// Fan out the TaskMessage to the union of regions declared by every
	// split's deployment. A worker subscribed to one of those regions will
	// pick up the message via its `filter_subject` and reconcile.
	// Previously this hardcoded `"global"`, which works for the wildcard
	// `edgecloud.tasks.>` subject but means no worker actually consumes
	// the message in a multi-region setup (their consumers are filtered
	// to their own region subject).
	regionSet := make(map[string]struct{}, len(splits))
	for _, sp := range splits {
		d, err := s.deploymentRepo.GetByID(ctx, sp.DeploymentID)
		if err != nil || d == nil {
			continue
		}
		for _, r := range domain.StringArrayTo(d.Regions) {
			regionSet[r] = struct{}{}
		}
	}
	regions := make([]string, 0, len(regionSet))
	for r := range regionSet {
		regions = append(regions, r)
	}
	if len(regions) == 0 {
		regions = []string{s.defaultRegion}
	}

	msg := &nats.TaskMessage{
		Type:      "task_update",
		Timestamp: time.Now(),
		TenantID:  tenantID,
		Apps: map[string]nats.AppConfig{
			appName: {
				DeploymentID:   splits[0].DeploymentID, // primary; Routes drives worker behavior
				DeploymentHash: primaryHash,
				Routes:         routes,
				Env:            envMap,
				Allowlist:      domain.StringArrayTo(tenant.AllowlistedDestinations),
				MaxMemoryMB:    maxMemoryMB,
			},
		},
	}

	var failedRegions []string
	for _, region := range regions {
		if err := s.publisher.PublishTaskUpdate(region, msg); err != nil {
			log.Printf("publishing task update for traffic split failed for region %q (tenant %s, app %s): %v", region, tenantID, appName, err)
			failedRegions = append(failedRegions, region)
		}
	}
	if len(failedRegions) > 0 {
		return fmt.Errorf("publishing traffic split failed for region(s): %s", strings.Join(failedRegions, ","))
	}
	return nil
}
