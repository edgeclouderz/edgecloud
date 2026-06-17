package service

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"strings"
	"testing"

	"github.com/edgeclouderz/edge-cloud/edge-control-plane/internal/domain"
)

// mockAPIKeyRepo implements apiKeyRepoInterface for testing.
type mockAPIKeyRepo struct {
	createFn         func(ctx context.Context, k *domain.APIKey) error
	getByHashFunc    func(ctx context.Context, hash string) (*domain.APIKey, error)
	listByTenantFn   func(ctx context.Context, tenantID string) ([]domain.APIKey, error)
	deleteFn         func(ctx context.Context, id string) error
	updateLastUsedFn func(ctx context.Context, id string) error
	updateHashFn     func(ctx context.Context, id, newHash, algo string) error
}

func (m *mockAPIKeyRepo) Create(ctx context.Context, k *domain.APIKey) error {
	if m.createFn == nil {
		return nil
	}
	return m.createFn(ctx, k)
}
func (m *mockAPIKeyRepo) GetByHash(ctx context.Context, hash string) (*domain.APIKey, error) {
	return m.getByHashFunc(ctx, hash)
}
func (m *mockAPIKeyRepo) ListByTenant(ctx context.Context, tenantID string) ([]domain.APIKey, error) {
	if m.listByTenantFn == nil {
		return nil, nil
	}
	return m.listByTenantFn(ctx, tenantID)
}
func (m *mockAPIKeyRepo) Delete(ctx context.Context, id string) error {
	if m.deleteFn == nil {
		return nil
	}
	return m.deleteFn(ctx, id)
}
func (m *mockAPIKeyRepo) UpdateLastUsed(ctx context.Context, id string) error {
	if m.updateLastUsedFn == nil {
		return nil
	}
	return m.updateLastUsedFn(ctx, id)
}
func (m *mockAPIKeyRepo) UpdateHash(ctx context.Context, id, newHash, algo string) error {
	if m.updateHashFn == nil {
		return nil
	}
	return m.updateHashFn(ctx, id, newHash, algo)
}

func sha256HexOf(s string) string {
	h := sha256.Sum256([]byte(s))
	return hex.EncodeToString(h[:])
}

func TestAPIKeyService_AuthenticateRawKey_Argon2_HappyPath(t *testing.T) {
	raw := "the-real-secret-key"
	hash, err := HashAPIKey(raw)
	if err != nil {
		t.Fatalf("HashAPIKey: %v", err)
	}
	shaHex := sha256HexOf(raw)

	lastUsedCalled := false
	repo := &mockAPIKeyRepo{
		getByHashFunc: func(ctx context.Context, h string) (*domain.APIKey, error) {
			if h != shaHex {
				t.Errorf("repo queried with wrong hash: got %q want %q", h, shaHex)
			}
			return &domain.APIKey{
				ID:            "k_test",
				TenantID:      "t_test",
				KeyHash:       hash,
				HashAlgorithm: domain.HashAlgorithmArgon2ID,
				Role:          domain.RoleDeveloper,
			}, nil
		},
		updateLastUsedFn: func(ctx context.Context, id string) error {
			lastUsedCalled = true
			return nil
		},
	}
	svc := NewAPIKeyService(nil)
	svc.apiKeyRepo = repo

	got, err := svc.AuthenticateRawKey(context.Background(), raw)
	if err != nil {
		t.Fatalf("AuthenticateRawKey: %v", err)
	}
	if got.ID != "k_test" {
		t.Errorf("ID = %q, want k_test", got.ID)
	}
	if !lastUsedCalled {
		t.Error("UpdateLastUsed was not called on successful auth")
	}
}

func TestAPIKeyService_AuthenticateRawKey_Argon2_WrongKey(t *testing.T) {
	hash, _ := HashAPIKey("the-real-key")
	repo := &mockAPIKeyRepo{
		getByHashFunc: func(ctx context.Context, h string) (*domain.APIKey, error) {
			return &domain.APIKey{ID: "k_test", KeyHash: hash, HashAlgorithm: domain.HashAlgorithmArgon2ID}, nil
		},
	}
	svc := NewAPIKeyService(nil)
	svc.apiKeyRepo = repo

	_, err := svc.AuthenticateRawKey(context.Background(), "the-wrong-key")
	if !errors.Is(err, ErrInvalidAPIKey) {
		t.Errorf("error = %v, want ErrInvalidAPIKey", err)
	}
}

func TestAPIKeyService_AuthenticateRawKey_LegacySHA256_LazyRehash(t *testing.T) {
	// Simulate a pre-migration row: stored as hex SHA-256.
	raw := "legacy-raw-key"
	legacyHash := sha256HexOf(raw)

	var rehashCalled bool
	var rehashToArgon bool
	var rehashID string
	repo := &mockAPIKeyRepo{
		getByHashFunc: func(ctx context.Context, h string) (*domain.APIKey, error) {
			return &domain.APIKey{
				ID:            "k_legacy",
				TenantID:      "t_test",
				KeyHash:       legacyHash,
				HashAlgorithm: domain.HashAlgorithmSHA256, // legacy
				Role:          domain.RoleDeveloper,
			}, nil
		},
		updateHashFn: func(ctx context.Context, id, newHash, algo string) error {
			rehashCalled = true
			rehashID = id
			if algo == domain.HashAlgorithmArgon2ID && strings.HasPrefix(newHash, "$argon2id$") {
				rehashToArgon = true
			}
			return nil
		},
	}
	svc := NewAPIKeyService(nil)
	svc.apiKeyRepo = repo

	got, err := svc.AuthenticateRawKey(context.Background(), raw)
	if err != nil {
		t.Fatalf("AuthenticateRawKey: %v", err)
	}
	if got.ID != "k_legacy" {
		t.Errorf("ID = %q, want k_legacy", got.ID)
	}
	if !rehashCalled {
		t.Error("UpdateHash was not called — legacy key was not lazily upgraded")
	}
	if !rehashToArgon {
		t.Error("rehash did not write argon2id format")
	}
	if rehashID != "k_legacy" {
		t.Errorf("rehash ID = %q, want k_legacy", rehashID)
	}
}

func TestAPIKeyService_AuthenticateRawKey_NoSuchKey(t *testing.T) {
	repo := &mockAPIKeyRepo{
		getByHashFunc: func(ctx context.Context, h string) (*domain.APIKey, error) {
			return nil, nil // not found
		},
	}
	svc := NewAPIKeyService(nil)
	svc.apiKeyRepo = repo

	_, err := svc.AuthenticateRawKey(context.Background(), "any-key")
	if !errors.Is(err, ErrInvalidAPIKey) {
		t.Errorf("error = %v, want ErrInvalidAPIKey", err)
	}
}

func TestAPIKeyService_AuthenticateRawKey_Empty(t *testing.T) {
	svc := NewAPIKeyService(nil)
	_, err := svc.AuthenticateRawKey(context.Background(), "")
	if !errors.Is(err, ErrInvalidAPIKey) {
		t.Errorf("error = %v, want ErrInvalidAPIKey", err)
	}
}
