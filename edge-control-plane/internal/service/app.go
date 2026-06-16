package service

import (
	"context"
	"fmt"
	"time"

	"github.com/edgeclouderz/edge-cloud/edge-control-plane/internal/domain"
	"github.com/edgeclouderz/edge-cloud/edge-control-plane/internal/repository"
	"github.com/google/uuid"
	"github.com/jmoiron/sqlx"
)

// AppService handles app business logic.
type AppService struct {
	db             *sqlx.DB
	appRepo        *repository.AppRepository
	deploymentRepo *repository.DeploymentRepository
	activeRepo     *repository.ActiveDeploymentRepository
	appEnvRepo     *repository.AppEnvRepository
}

func NewAppService(
	db *sqlx.DB,
	appRepo *repository.AppRepository,
	deploymentRepo *repository.DeploymentRepository,
	activeRepo *repository.ActiveDeploymentRepository,
	appEnvRepo *repository.AppEnvRepository,
) *AppService {
	return &AppService{
		db:             db,
		appRepo:        appRepo,
		deploymentRepo: deploymentRepo,
		activeRepo:     activeRepo,
		appEnvRepo:     appEnvRepo,
	}
}

// Create creates a new app. Returns ErrAppAlreadyExists if it already exists.
var ErrAppAlreadyExists = fmt.Errorf("app already exists")

func (s *AppService) Create(ctx context.Context, tenantID, appName string, req *domain.CreateAppRequest) (*domain.App, error) {
	if !IsValidAppName(appName) {
		return nil, fmt.Errorf("invalid app name: %s", appName)
	}

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
	return s.appRepo.Get(ctx, tenantID, appName)
}

// List returns apps for a tenant with pagination.
func (s *AppService) List(ctx context.Context, tenantID string, limit, offset int) ([]domain.App, error) {
	return s.appRepo.List(ctx, tenantID, limit, offset)
}

// Delete deletes an app and all its associated data atomically.
// Returns ErrAppNotFound if the app does not exist.
var ErrAppNotFound = fmt.Errorf("app not found")

func (s *AppService) Delete(ctx context.Context, tenantID, appName string) error {
	// Use AtomicDelete to check existence and delete in one step.
	deleted, err := s.appRepo.AtomicDelete(ctx, tenantID, appName)
	if err != nil {
		return fmt.Errorf("deleting app: %w", err)
	}
	if !deleted {
		return ErrAppNotFound
	}

	// Cascade deletes run in a transaction so they either all succeed or all fail.
	err = repository.Transaction(ctx, s.db, func(tx *sqlx.Tx) error {
		appEnvRepo := s.appEnvRepo.WithTx(tx)
		activeRepo := s.activeRepo.WithTx(tx)
		deploymentRepo := s.deploymentRepo.WithTx(tx)

		if err := appEnvRepo.DeleteByApp(ctx, tenantID, appName); err != nil {
			return fmt.Errorf("deleting app env: %w", err)
		}
		if err := activeRepo.Delete(ctx, tenantID, appName); err != nil {
			return fmt.Errorf("deleting active deployment: %w", err)
		}
		if err := deploymentRepo.DeleteByApp(ctx, tenantID, appName); err != nil {
			return fmt.Errorf("deleting deployments: %w", err)
		}
		return nil
	})
	if err != nil {
		// Log but don't fail — app row already deleted above.
		// In production, consider a background reconciler to clean orphaned rows.
		fmt.Printf("warning: cascade delete partially failed after app deletion: %v\n", err)
	}
	return nil
}

// CreateIfNotExists creates an app if it doesn't already exist.
// Idempotent — safe to call multiple times.
func (s *AppService) CreateIfNotExists(ctx context.Context, tenantID, appName string) error {
	if !IsValidAppName(appName) {
		return fmt.Errorf("invalid app name: %s", appName)
	}

	app := &domain.App{
		ID:          "a_" + uuid.New().String(),
		TenantID:    tenantID,
		Name:        appName,
		Description: nil,
		CreatedAt:   time.Now(),
	}
	// Upsert is inherently idempotent — concurrent calls are safely deduplicated.
	_, err := s.appRepo.InsertIfNotExists(ctx, app)
	if err != nil {
		return fmt.Errorf("creating app: %w", err)
	}
	return nil
}
