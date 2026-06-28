package service

import (
	"context"
	"errors"
	"testing"

	"github.com/edgeclouderz/edge-cloud/edge-control-plane/internal/domain"
)

type mockEnvRepo struct {
	setFn    func(ctx context.Context, env *domain.AppEnv) error
	listFn   func(ctx context.Context, tenantID, appName string) ([]domain.AppEnv, error)
	deleteFn func(ctx context.Context, tenantID, appName, key string) error
}

func (m *mockEnvRepo) Set(ctx context.Context, env *domain.AppEnv) error {
	return m.setFn(ctx, env)
}
func (m *mockEnvRepo) List(ctx context.Context, tenantID, appName string) ([]domain.AppEnv, error) {
	return m.listFn(ctx, tenantID, appName)
}
func (m *mockEnvRepo) Delete(ctx context.Context, tenantID, appName, key string) error {
	return m.deleteFn(ctx, tenantID, appName, key)
}

func newEnvSvc(repo *mockEnvRepo) *EnvService {
	return NewEnvService(repo)
}

func TestEnvService_SetEnv(t *testing.T) {
	var called bool
	var capturedEnv domain.AppEnv
	repo := &mockEnvRepo{
		setFn: func(ctx context.Context, env *domain.AppEnv) error {
			called = true
			capturedEnv = *env
			return nil
		},
	}
	svc := newEnvSvc(repo)

	if err := svc.SetEnv(context.Background(), "t_1", "hello", "LOG_LEVEL", "debug"); err != nil {
		t.Fatalf("SetEnv: %v", err)
	}
	if !called {
		t.Fatal("repo.Set was not called")
	}
	if capturedEnv.TenantID != "t_1" || capturedEnv.AppName != "hello" || capturedEnv.EnvKey != "LOG_LEVEL" || capturedEnv.EnvValue != "debug" {
		t.Errorf("env = %+v", capturedEnv)
	}
}

func TestEnvService_ListEnv(t *testing.T) {
	repo := &mockEnvRepo{
		listFn: func(ctx context.Context, tenantID, appName string) ([]domain.AppEnv, error) {
			return []domain.AppEnv{
				{TenantID: "t_1", AppName: "hello", EnvKey: "K", EnvValue: "V"},
			}, nil
		},
	}
	svc := newEnvSvc(repo)

	envs, err := svc.ListEnv(context.Background(), "t_1", "hello")
	if err != nil {
		t.Fatalf("ListEnv: %v", err)
	}
	if len(envs) != 1 || envs[0].EnvKey != "K" {
		t.Errorf("envs = %+v", envs)
	}
}

func TestEnvService_DeleteEnv(t *testing.T) {
	var calledKey string
	repo := &mockEnvRepo{
		deleteFn: func(ctx context.Context, tenantID, appName, key string) error {
			calledKey = key
			return nil
		},
	}
	svc := newEnvSvc(repo)

	if err := svc.DeleteEnv(context.Background(), "t_1", "hello", "LOG_LEVEL"); err != nil {
		t.Fatalf("DeleteEnv: %v", err)
	}
	if calledKey != "LOG_LEVEL" {
		t.Errorf("delete key = %q, want LOG_LEVEL", calledKey)
	}
}

func TestEnvService_SetEnv_PropagatesError(t *testing.T) {
	want := errors.New("db down")
	repo := &mockEnvRepo{setFn: func(ctx context.Context, env *domain.AppEnv) error { return want }}
	svc := newEnvSvc(repo)

	err := svc.SetEnv(context.Background(), "t_1", "hello", "K", "V")
	if !errors.Is(err, want) {
		t.Errorf("error = %v, want %v", err, want)
	}
}

func TestEnvService_ListEnv_PropagatesError(t *testing.T) {
	want := errors.New("db down")
	repo := &mockEnvRepo{listFn: func(ctx context.Context, tenantID, appName string) ([]domain.AppEnv, error) { return nil, want }}
	svc := newEnvSvc(repo)

	_, err := svc.ListEnv(context.Background(), "t_1", "hello")
	if !errors.Is(err, want) {
		t.Errorf("error = %v, want %v", err, want)
	}
}
