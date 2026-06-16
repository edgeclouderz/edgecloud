package service

import (
	"context"
	"fmt"
	"time"

	"github.com/edgeclouderz/edge-cloud/edge-control-plane/internal/domain"
	"github.com/edgeclouderz/edge-cloud/edge-control-plane/internal/repository"
	"github.com/google/uuid"
)

// AppService handles app business logic.
type AppService struct {
	appRepo        *repository.AppRepository
	deploymentRepo *repository.DeploymentRepository
	activeRepo     *repository.ActiveDeploymentRepository
	appEnvRepo     *repository.AppEnvRepository
}

func NewAppService(
	appRepo *repository.AppRepository,
	deploymentRepo *repository.DeploymentRepository,
	activeRepo *repository.ActiveDeploymentRepository,
	appEnvRepo *repository.AppEnvRepository,
) *AppService {
	return &AppService{
		appRepo:        appRepo,
		deploymentRepo: deploymentRepo,
		activeRepo:     activeRepo,
		appEnvRepo:     appEnvRepo,
	}
}

// Create creates a new app. Returns ErrAppAlreadyExists if it already exists.
var ErrAppAlreadyExists = fmt.Errorf("app already exists")

func (s *AppService) Create(ctx context.Context, tenantID, appName string, req *domain.CreateAppRequest) (*domain.App, error) {
	// Check if app already exists
	exists, err := s.appRepo.Exists(ctx, tenantID, appName)
	if err != nil {
		return nil, fmt.Errorf("checking app existence: %w", err)
	}
	if exists {
		return nil, ErrAppAlreadyExists
	}

	var desc *string
	if req.Description != "" {
		desc = &req.Description
	}

	app := &domain.App{
		ID:          "a_" + uuid.New().String(),
		TenantID:    tenantID,
		Name:        appName,
		Description: desc,
		CreatedAt:   time.Now(),
	}
	if err := s.appRepo.Create(ctx, app); err != nil {
		return nil, fmt.Errorf("creating app: %w", err)
	}
	return app, nil
}

// Get returns an app by name, or nil if not found.
func (s *AppService) Get(ctx context.Context, tenantID, appName string) (*domain.App, error) {
	app, err := s.appRepo.Get(ctx, tenantID, appName)
	if err != nil {
		return nil, err
	}
	return app, nil
}

// List returns all apps for a tenant.
func (s *AppService) List(ctx context.Context, tenantID string) ([]domain.App, error) {
	return s.appRepo.List(ctx, tenantID)
}

// Delete deletes an app and all its associated data (deployments, active deployment, env vars).
// Returns ErrAppNotFound if the app does not exist.
var ErrAppNotFound = fmt.Errorf("app not found")

func (s *AppService) Delete(ctx context.Context, tenantID, appName string) error {
	// Check app exists
	exists, err := s.appRepo.Exists(ctx, tenantID, appName)
	if err != nil {
		return fmt.Errorf("checking app existence: %w", err)
	}
	if !exists {
		return ErrAppNotFound
	}

	// Delete in order: env vars, active deployments, deployments, then app.
	if err := s.appEnvRepo.DeleteByApp(ctx, tenantID, appName); err != nil {
		return fmt.Errorf("deleting app env: %w", err)
	}
	if err := s.activeRepo.Delete(ctx, tenantID, appName); err != nil {
		return fmt.Errorf("deleting active deployment: %w", err)
	}
	if err := s.deploymentRepo.DeleteByApp(ctx, tenantID, appName); err != nil {
		return fmt.Errorf("deleting deployments: %w", err)
	}
	if err := s.appRepo.Delete(ctx, tenantID, appName); err != nil {
		return fmt.Errorf("deleting app: %w", err)
	}
	return nil
}

// CreateIfNotExists creates an app if it doesn't already exist.
// This is called by Deploy to ensure the app record exists before deploying.
// It is safe to call multiple times — idempotent.
func (s *AppService) CreateIfNotExists(ctx context.Context, tenantID, appName string) error {
	exists, err := s.appRepo.Exists(ctx, tenantID, appName)
	if err != nil {
		return fmt.Errorf("checking app existence: %w", err)
	}
	if exists {
		return nil
	}

	app := &domain.App{
		ID:          "a_" + uuid.New().String(),
		TenantID:    tenantID,
		Name:        appName,
		Description: nil,
		CreatedAt:   time.Now(),
	}
	// Ignore ErrAppAlreadyExists — another request may have created it concurrently.
	if err := s.appRepo.Create(ctx, app); err != nil {
		// Check if it was a concurrent creation
		exists, err2 := s.appRepo.Exists(ctx, tenantID, appName)
		if err2 == nil && exists {
			return nil // raced with another creator, app now exists
		}
		return fmt.Errorf("creating app: %w", err)
	}
	return nil
}
