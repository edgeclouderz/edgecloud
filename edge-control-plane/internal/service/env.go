package service

import (
	"context"

	"github.com/edgeclouderz/edge-cloud/edge-control-plane/internal/domain"
)

// EnvRepoInterface is the subset of *repository.AppEnvRepository used by EnvService.
type EnvRepoInterface interface {
	Set(ctx context.Context, env *domain.AppEnv) error
	List(ctx context.Context, tenantID, appName string) ([]domain.AppEnv, error)
	Delete(ctx context.Context, tenantID, appName, key string) error
}

// EnvService handles environment variable business logic.
type EnvService struct {
	appEnvRepo EnvRepoInterface
}

func NewEnvService(appEnvRepo EnvRepoInterface) *EnvService {
	return &EnvService{appEnvRepo: appEnvRepo}
}

func (s *EnvService) SetEnv(ctx context.Context, tenantID, appName, key, value string) error {
	return s.appEnvRepo.Set(ctx, &domain.AppEnv{
		TenantID: tenantID,
		AppName:  appName,
		EnvKey:   key,
		EnvValue: value,
	})
}

func (s *EnvService) ListEnv(ctx context.Context, tenantID, appName string) ([]domain.AppEnv, error) {
	return s.appEnvRepo.List(ctx, tenantID, appName)
}

func (s *EnvService) DeleteEnv(ctx context.Context, tenantID, appName, key string) error {
	return s.appEnvRepo.Delete(ctx, tenantID, appName, key)
}
