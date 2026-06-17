package service

import (
	"context"
	"crypto/rand"
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"fmt"
	"log"

	"github.com/edgeclouderz/edge-cloud/edge-control-plane/internal/domain"
	"github.com/edgeclouderz/edge-cloud/edge-control-plane/internal/repository"
	"github.com/google/uuid"
)

// ErrInvalidAPIKey is returned by AuthenticateRawKey when the raw key does not
// match any row. Callers should map this to 401 Unauthorized.
var ErrInvalidAPIKey = errors.New("invalid api key")

// apiKeyRepoInterface is the subset of *repository.APIKeyRepository used by
// APIKeyService. Defining it here keeps the service decoupled from the
// concrete repo and lets tests substitute a mock.
type apiKeyRepoInterface interface {
	Create(ctx context.Context, k *domain.APIKey) error
	GetByHash(ctx context.Context, hash string) (*domain.APIKey, error)
	ListByTenant(ctx context.Context, tenantID string) ([]domain.APIKey, error)
	Delete(ctx context.Context, id string) error
	UpdateLastUsed(ctx context.Context, id string) error
	UpdateHash(ctx context.Context, id, newHash, algo string) error
}

// APIKeyService handles API key business logic.
type APIKeyService struct {
	apiKeyRepo apiKeyRepoInterface
}

func NewAPIKeyService(apiKeyRepo *repository.APIKeyRepository) *APIKeyService {
	return &APIKeyService{apiKeyRepo: apiKeyRepo}
}

// CreateAPIKey creates a new API key and returns the raw key (shown only once).
//
// Keys are stored as argon2id hashes (PHC-formatted). The raw key is returned
// to the caller exactly once and is never persisted.
func (s *APIKeyService) CreateAPIKey(ctx context.Context, tenantID, name, role string) (*domain.APIKey, string, error) {
	if !domain.IsValidRole(role) {
		return nil, "", fmt.Errorf("invalid role: %s", role)
	}

	// Generate a 32-byte random key, hex-encoded for transport (64 chars).
	raw := make([]byte, 32)
	if _, err := rand.Read(raw); err != nil {
		return nil, "", fmt.Errorf("generating key: %w", err)
	}
	rawKey := hex.EncodeToString(raw)

	keyHash, err := HashAPIKey(rawKey)
	if err != nil {
		return nil, "", fmt.Errorf("hashing key: %w", err)
	}

	apiKey := &domain.APIKey{
		ID:            "k_" + uuid.New().String(),
		TenantID:      tenantID,
		Name:          name,
		KeyHash:       keyHash,
		Role:          role,
		HashAlgorithm: domain.HashAlgorithmArgon2ID,
	}

	if err := s.apiKeyRepo.Create(ctx, apiKey); err != nil {
		return nil, "", fmt.Errorf("creating api key: %w", err)
	}

	return apiKey, rawKey, nil
}

func (s *APIKeyService) ListAPIKeys(ctx context.Context, tenantID string) ([]domain.APIKey, error) {
	return s.apiKeyRepo.ListByTenant(ctx, tenantID)
}

func (s *APIKeyService) DeleteAPIKey(ctx context.Context, id string) error {
	return s.apiKeyRepo.Delete(ctx, id)
}

// AuthenticateRawKey looks up a key by hashing the raw value with SHA-256 and
// dispatching to the algorithm stored in the row.
//
// New keys are stored as argon2id (PHC-formatted); legacy keys created before
// migration 005 keep their hex SHA-256 hash. On successful verification of a
// legacy row the function transparently rehashes the key with argon2id and
// persists the upgrade, so callers don't need to do anything special and the
// migration finishes organically as each key is next used.
func (s *APIKeyService) AuthenticateRawKey(ctx context.Context, rawKey string) (*domain.APIKey, error) {
	if rawKey == "" {
		return nil, ErrInvalidAPIKey
	}

	sha := sha256.Sum256([]byte(rawKey))
	shaHex := hex.EncodeToString(sha[:])

	candidate, err := s.apiKeyRepo.GetByHash(ctx, shaHex)
	if err != nil {
		return nil, fmt.Errorf("looking up api key: %w", err)
	}
	if candidate == nil {
		return nil, ErrInvalidAPIKey
	}

	algo := candidate.HashAlgorithm
	if algo == "" {
		// Pre-migration rows. Treat as legacy SHA-256.
		algo = domain.HashAlgorithmSHA256
	}

	switch algo {
	case domain.HashAlgorithmSHA256:
		// Legacy fast path: the hex SHA-256 already matched the stored hash
		// (that's how GetByHash found the row), so verification succeeds.
		// Lazily upgrade to argon2id.
		newHash, hashErr := HashAPIKey(rawKey)
		if hashErr != nil {
			log.Printf("warning: lazy rehash failed for key %s: %v", candidate.ID, hashErr)
		} else if err := s.apiKeyRepo.UpdateHash(ctx, candidate.ID, newHash, domain.HashAlgorithmArgon2ID); err != nil {
			log.Printf("warning: failed to persist lazy rehash for key %s: %v", candidate.ID, err)
		}

	case domain.HashAlgorithmArgon2ID:
		ok, err := VerifyAPIKey(rawKey, candidate.KeyHash)
		if err != nil {
			return nil, fmt.Errorf("verifying api key: %w", err)
		}
		if !ok {
			return nil, ErrInvalidAPIKey
		}

	default:
		return nil, fmt.Errorf("unsupported hash algorithm %q for key %s", algo, candidate.ID)
	}

	if err := s.apiKeyRepo.UpdateLastUsed(ctx, candidate.ID); err != nil {
		log.Printf("warning: failed to update last_used for api key %s: %v", candidate.ID, err)
	}
	return candidate, nil
}
