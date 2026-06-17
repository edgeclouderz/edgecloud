package repository

import (
	"context"
	"database/sql"
	"time"

	"github.com/edgeclouderz/edge-cloud/edge-control-plane/internal/domain"
	"github.com/jmoiron/sqlx"
)

// APIKeyRepository handles API key data access.
type APIKeyRepository struct {
	db DBTX
}

func NewAPIKeyRepository(db *sqlx.DB) *APIKeyRepository {
	return &APIKeyRepository{db: db}
}

// WithTx returns a new APIKeyRepository using the provided transaction.
func (r *APIKeyRepository) WithTx(tx *sqlx.Tx) *APIKeyRepository {
	return &APIKeyRepository{db: tx}
}

func (r *APIKeyRepository) Create(ctx context.Context, k *domain.APIKey) error {
	query := `INSERT INTO api_keys (id, tenant_id, name, key_hash, lookup_hash, role, created_at, expires_at, hash_algorithm) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)`
	algo := k.HashAlgorithm
	if algo == "" {
		algo = domain.HashAlgorithmArgon2ID // safe default for new keys
	}
	_, err := r.db.ExecContext(ctx, query, k.ID, k.TenantID, k.Name, k.KeyHash, k.LookupHash, k.Role, k.CreatedAt, k.ExpiresAt, algo)
	return err
}

func (r *APIKeyRepository) GetByID(ctx context.Context, id string) (*domain.APIKey, error) {
	var k domain.APIKey
	query := `SELECT id, tenant_id, name, key_hash, lookup_hash, role, created_at, last_used, expires_at, hash_algorithm FROM api_keys WHERE id = $1`
	err := r.db.GetContext(ctx, &k, query, id)
	if err == sql.ErrNoRows {
		return nil, nil
	}
	return &k, err
}

// GetByLookupHash fetches the key row by its stable SHA-256 lookup hash.
// This is the path AuthenticateRawKey uses to find candidate rows before
// dispatching to the algorithm-specific verifier. (See migration 006.)
func (r *APIKeyRepository) GetByLookupHash(ctx context.Context, lookupHash string) (*domain.APIKey, error) {
	var k domain.APIKey
	query := `SELECT id, tenant_id, name, key_hash, lookup_hash, role, created_at, last_used, expires_at, hash_algorithm FROM api_keys WHERE lookup_hash = $1`
	err := r.db.GetContext(ctx, &k, query, lookupHash)
	if err == sql.ErrNoRows {
		return nil, nil
	}
	return &k, err
}

func (r *APIKeyRepository) ListByTenant(ctx context.Context, tenantID string) ([]domain.APIKey, error) {
	var keys []domain.APIKey
	query := `SELECT id, tenant_id, name, key_hash, lookup_hash, role, created_at, last_used, expires_at, hash_algorithm FROM api_keys WHERE tenant_id = $1 ORDER BY created_at DESC`
	err := r.db.SelectContext(ctx, &keys, query, tenantID)
	return keys, err
}

func (r *APIKeyRepository) Delete(ctx context.Context, id string) error {
	_, err := r.db.ExecContext(ctx, `DELETE FROM api_keys WHERE id = $1`, id)
	return err
}

func (r *APIKeyRepository) UpdateLastUsed(ctx context.Context, id string) error {
	_, err := r.db.ExecContext(ctx, `UPDATE api_keys SET last_used = $2 WHERE id = $1`, id, time.Now())
	return err
}

// UpdateHash overwrites the stored hash and algorithm for a key. Used by the
// AuthMiddleware lazy-rehash path on first successful auth of a legacy
// SHA-256-stored key.
func (r *APIKeyRepository) UpdateHash(ctx context.Context, id, newHash, algo string) error {
	_, err := r.db.ExecContext(ctx, `UPDATE api_keys SET key_hash = $2, hash_algorithm = $3 WHERE id = $1`, id, newHash, algo)
	return err
}

// UpdateHashIfAlgorithm atomically overwrites key_hash and hash_algorithm only
// if the row's current hash_algorithm matches currentAlgo. Returns the number
// of rows updated so the caller can detect "another auth won the race".
//
// Used by the lazy-rehash path: only the request whose CAS guard matches
// "sha256" actually writes the new argon2id hash. Concurrent requests whose
// CAS loses silently observe the row in its upgraded state and skip the
// overwrite, avoiding the random-salt ping-pong that would otherwise happen.
func (r *APIKeyRepository) UpdateHashIfAlgorithm(
	ctx context.Context, id, currentAlgo, newHash, newAlgo string,
) (int64, error) {
	res, err := r.db.ExecContext(ctx,
		`UPDATE api_keys SET key_hash = $3, hash_algorithm = $4 WHERE id = $1 AND hash_algorithm = $2`,
		id, currentAlgo, newHash, newAlgo,
	)
	if err != nil {
		return 0, err
	}
	return res.RowsAffected()
}
