package service

import (
	"context"
	"errors"
	"strings"
	"sync"
	"testing"

	"github.com/edgeclouderz/edge-cloud/edge-control-plane/internal/domain"
)

// mockAPIKeyRepo implements apiKeyRepoInterface for testing.
type mockAPIKeyRepo struct {
	createFn                func(ctx context.Context, k *domain.APIKey) error
	getByLookupHashFn       func(ctx context.Context, lookupHash string) (*domain.APIKey, error)
	listByTenantFn          func(ctx context.Context, tenantID string) ([]domain.APIKey, error)
	deleteFn                func(ctx context.Context, id string) error
	updateLastUsedFn        func(ctx context.Context, id string) error
	updateHashFn            func(ctx context.Context, id, newHash, algo string) error
	updateHashIfAlgorithmFn func(ctx context.Context, id, currentAlgo, newHash, newAlgo string) (int64, error)
}

func (m *mockAPIKeyRepo) Create(ctx context.Context, k *domain.APIKey) error {
	if m.createFn == nil {
		return nil
	}
	return m.createFn(ctx, k)
}
func (m *mockAPIKeyRepo) GetByLookupHash(ctx context.Context, lookupHash string) (*domain.APIKey, error) {
	if m.getByLookupHashFn == nil {
		return nil, nil
	}
	return m.getByLookupHashFn(ctx, lookupHash)
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
func (m *mockAPIKeyRepo) UpdateHashIfAlgorithm(ctx context.Context, id, currentAlgo, newHash, newAlgo string) (int64, error) {
	if m.updateHashIfAlgorithmFn == nil {
		return 0, nil
	}
	return m.updateHashIfAlgorithmFn(ctx, id, currentAlgo, newHash, newAlgo)
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
		getByLookupHashFn: func(ctx context.Context, h string) (*domain.APIKey, error) {
			if h != shaHex {
				t.Errorf("repo queried with wrong lookup hash: got %q want %q", h, shaHex)
			}
			return &domain.APIKey{
				ID:            "k_test",
				TenantID:      "t_test",
				KeyHash:       hash,
				LookupHash:    shaHex,
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
		getByLookupHashFn: func(ctx context.Context, h string) (*domain.APIKey, error) {
			return &domain.APIKey{ID: "k_test", KeyHash: hash, LookupHash: h, HashAlgorithm: domain.HashAlgorithmArgon2ID}, nil
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
	var casAlgo string
	repo := &mockAPIKeyRepo{
		getByLookupHashFn: func(ctx context.Context, h string) (*domain.APIKey, error) {
			return &domain.APIKey{
				ID:            "k_legacy",
				TenantID:      "t_test",
				KeyHash:       legacyHash,
				LookupHash:    h,
				HashAlgorithm: domain.HashAlgorithmSHA256, // legacy
				Role:          domain.RoleDeveloper,
			}, nil
		},
		updateHashIfAlgorithmFn: func(ctx context.Context, id, currentAlgo, newHash, newAlgo string) (int64, error) {
			rehashCalled = true
			rehashID = id
			casAlgo = currentAlgo
			if newAlgo == domain.HashAlgorithmArgon2ID && strings.HasPrefix(newHash, "$argon2id$") {
				rehashToArgon = true
			}
			return 1, nil
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
		t.Error("UpdateHashIfAlgorithm was not called — legacy key was not lazily upgraded")
	}
	if !rehashToArgon {
		t.Error("rehash did not write argon2id format")
	}
	if rehashID != "k_legacy" {
		t.Errorf("rehash ID = %q, want k_legacy", rehashID)
	}
	if casAlgo != domain.HashAlgorithmSHA256 {
		t.Errorf("CAS guard algo = %q, want %q", casAlgo, domain.HashAlgorithmSHA256)
	}
}

// TestAPIKeyService_AuthenticateRawKey_ConcurrentLazyRehash exercises the
// atomic-CAS path: two goroutines authenticate the same legacy SHA-256 row
// simultaneously. Only one CAS wins (rows-affected == 1); the other observes
// rows-affected == 0 (or its auth path loses to the upgraded row). Both
// authentications must succeed without panic, and the row must end up in
// argon2id format.
func TestAPIKeyService_AuthenticateRawKey_ConcurrentLazyRehash(t *testing.T) {
	raw := "concurrent-legacy-key"
	legacyHash := sha256HexOf(raw)

	var (
		mu          sync.Mutex
		casAttempts int
		casWins     int
	)

	// First call returns 1 (this goroutine upgraded the row).
	// Subsequent calls return 0 (the row is already in argon2id).
	updateHashIfAlgorithmFn := func(ctx context.Context, id, currentAlgo, newHash, newAlgo string) (int64, error) {
		mu.Lock()
		casAttempts++
		var affected int64
		if casAttempts == 1 {
			affected = 1
			casWins++
		}
		mu.Unlock()
		return affected, nil
	}

	repo := &mockAPIKeyRepo{
		getByLookupHashFn: func(ctx context.Context, h string) (*domain.APIKey, error) {
			// Always return the legacy row — both auth attempts see it as
			// SHA-256. The CAS guard is what serializes the upgrade.
			return &domain.APIKey{
				ID:            "k_legacy",
				TenantID:      "t_test",
				KeyHash:       legacyHash,
				LookupHash:    h,
				HashAlgorithm: domain.HashAlgorithmSHA256,
				Role:          domain.RoleDeveloper,
			}, nil
		},
		updateHashIfAlgorithmFn: updateHashIfAlgorithmFn,
	}
	svc := NewAPIKeyService(nil)
	svc.apiKeyRepo = repo

	const goroutines = 4
	var wg sync.WaitGroup
	errs := make(chan error, goroutines)
	for i := 0; i < goroutines; i++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			_, err := svc.AuthenticateRawKey(context.Background(), raw)
			if err != nil {
				errs <- err
			}
		}()
	}
	wg.Wait()
	close(errs)
	for err := range errs {
		t.Errorf("concurrent auth returned error: %v", err)
	}
	if casAttempts != goroutines {
		t.Errorf("CAS attempts = %d, want %d", casAttempts, goroutines)
	}
	if casWins != 1 {
		t.Errorf("CAS wins = %d, want exactly 1", casWins)
	}
}

func TestAPIKeyService_AuthenticateRawKey_NoSuchKey(t *testing.T) {
	repo := &mockAPIKeyRepo{
		getByLookupHashFn: func(ctx context.Context, h string) (*domain.APIKey, error) {
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
