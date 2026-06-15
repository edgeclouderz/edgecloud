package service

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/hex"
	"fmt"
	"io"
	"time"

	"github.com/edgeclouderz/edge-cloud/edge-control-plane/internal/domain"
	"github.com/edgeclouderz/edge-cloud/edge-control-plane/internal/nats"
	"github.com/edgeclouderz/edge-cloud/edge-control-plane/internal/repository"
	"github.com/edgeclouderz/edge-cloud/edge-control-plane/internal/storage"
	"github.com/google/uuid"
)

// DeploymentService handles deployment business logic.
type DeploymentService struct {
	deploymentRepo *repository.DeploymentRepository
	activeRepo     *repository.ActiveDeploymentRepository
	appEnvRepo     *repository.AppEnvRepository
	quotaRepo      *repository.QuotaRepository
	tenantRepo     *repository.TenantRepository
	artifactStore  *storage.ArtifactStore
	publisher      nats.Publisher
}

func NewDeploymentService(
	deploymentRepo *repository.DeploymentRepository,
	activeRepo *repository.ActiveDeploymentRepository,
	appEnvRepo *repository.AppEnvRepository,
	quotaRepo *repository.QuotaRepository,
	tenantRepo *repository.TenantRepository,
	artifactStore *storage.ArtifactStore,
	publisher nats.Publisher,
) *DeploymentService {
	return &DeploymentService{
		deploymentRepo: deploymentRepo,
		activeRepo:     activeRepo,
		appEnvRepo:     appEnvRepo,
		quotaRepo:      quotaRepo,
		tenantRepo:     tenantRepo,
		artifactStore:  artifactStore,
		publisher:      publisher,
	}
}

// Deploy creates a new deployment and stores the artifact.
func (s *DeploymentService) Deploy(ctx context.Context, tenantID, appName string, r io.Reader) (*domain.Deployment, error) {
	// Check quota
	quota, err := s.quotaRepo.GetByTenantID(ctx, tenantID)
	if err != nil {
		return nil, fmt.Errorf("getting quota: %w", err)
	}

	count, err := s.deploymentRepo.CountByApp(ctx, tenantID, appName)
	if err != nil {
		return nil, fmt.Errorf("counting deployments: %w", err)
	}
	if count >= quota.MaxDeployments {
		return nil, fmt.Errorf("max deployments (%d) reached", quota.MaxDeployments)
	}

	// Read artifact and compute hash
	data, err := io.ReadAll(r)
	if err != nil {
		return nil, fmt.Errorf("reading artifact: %w", err)
	}
	hash := sha256.Sum256(data)

	deployment := &domain.Deployment{
		ID:        "d_" + uuid.New().String(),
		TenantID:  tenantID,
		AppName:   appName,
		Status:    domain.StatusDeployed,
		Hash:      hex.EncodeToString(hash[:]),
		CreatedAt: time.Now(),
	}

	if err := s.deploymentRepo.Create(ctx, deployment); err != nil {
		return nil, fmt.Errorf("creating deployment: %w", err)
	}

	// Save artifact
	if err := s.artifactStore.Save(tenantID, appName, deployment.ID, bytes.NewReader(data)); err != nil {
		return nil, fmt.Errorf("saving artifact: %w", err)
	}

	return deployment, nil
}

func (s *DeploymentService) GetDeployment(ctx context.Context, id string) (*domain.Deployment, error) {
	return s.deploymentRepo.GetByID(ctx, id)
}

func (s *DeploymentService) ListDeployments(ctx context.Context, tenantID, appName string) ([]domain.Deployment, error) {
	return s.deploymentRepo.ListByApp(ctx, tenantID, appName)
}

func (s *DeploymentService) ActivateDeployment(ctx context.Context, tenantID, appName, deploymentID string) error {
	deployment, err := s.deploymentRepo.GetByID(ctx, deploymentID)
	if err != nil || deployment == nil {
		return fmt.Errorf("deployment not found")
	}
	if deployment.TenantID != tenantID || deployment.AppName != appName {
		return fmt.Errorf("deployment not found")
	}

	if err := s.activeRepo.Set(ctx, &domain.ActiveDeployment{
		TenantID:     tenantID,
		AppName:      appName,
		DeploymentID: deploymentID,
	}); err != nil {
		return fmt.Errorf("setting active deployment: %w", err)
	}

	// Publish task update
	envs, _ := s.appEnvRepo.List(ctx, tenantID, appName)
	envMap := make(map[string]string)
	for _, e := range envs {
		envMap[e.EnvKey] = e.EnvValue
	}

	msg := &nats.TaskMessage{
		Type:      "task_update",
		Timestamp: time.Now(),
		TenantID:  tenantID,
		Apps: map[string]nats.AppConfig{
			appName: {
				DeploymentID:   deploymentID,
				DeploymentHash: deployment.Hash,
				Env:            envMap,
				Allowlist:      []string{},
			},
		},
	}
	_ = s.publisher.PublishTaskUpdate("global", msg) // region would come from worker selection

	return nil
}

func (s *DeploymentService) GetActiveDeployment(ctx context.Context, tenantID, appName string) (*domain.Deployment, error) {
	ad, err := s.activeRepo.Get(ctx, tenantID, appName)
	if err != nil || ad == nil {
		return nil, err
	}
	return s.deploymentRepo.GetByID(ctx, ad.DeploymentID)
}

func (s *DeploymentService) GetArtifact(ctx context.Context, tenantID, appName, deploymentID string) (io.ReadCloser, error) {
	return s.artifactStore.Open(tenantID, appName, deploymentID)
}
