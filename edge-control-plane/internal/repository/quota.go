package repository

import (
	"context"
	"database/sql"

	"github.com/edgeclouderz/edge-cloud/edge-control-plane/internal/domain"
	"github.com/jmoiron/sqlx"
)

// QuotaRepository handles quota data access.
type QuotaRepository struct {
	db DBTX
}

func NewQuotaRepository(db *sqlx.DB) *QuotaRepository {
	return &QuotaRepository{db: db}
}

// WithTx returns a new QuotaRepository using the provided transaction.
func (r *QuotaRepository) WithTx(tx *sqlx.Tx) *QuotaRepository {
	return &QuotaRepository{db: tx}
}

func (r *QuotaRepository) Create(ctx context.Context, q *domain.Quota) error {
	query := `INSERT INTO quotas (tenant_id, max_deployments, max_apps, max_workers, max_memory_mb, max_outbound_mb) VALUES ($1, $2, $3, $4, $5, $6)`
	_, err := r.db.ExecContext(ctx, query, q.TenantID, q.MaxDeployments, q.MaxApps, q.MaxWorkers, q.MaxMemoryMB, q.MaxOutboundMB)
	return err
}

func (r *QuotaRepository) GetByTenantID(ctx context.Context, tenantID string) (*domain.Quota, error) {
	var q domain.Quota
	query := `SELECT tenant_id, max_deployments, max_apps, max_workers, max_memory_mb, max_outbound_mb, used_outbound_bytes, quota_period_start FROM quotas WHERE tenant_id = $1`
	err := r.db.GetContext(ctx, &q, query, tenantID)
	if err == sql.ErrNoRows {
		return nil, nil
	}
	return &q, err
}

// AddOutboundBytes atomically accumulates delta into used_outbound_bytes and
// returns the updated quota row. When the stored quota_period_start is in a
// past calendar month (UTC), the counter and period are reset first so the
// monthly cap applies to the current month only — no separate cron required.
func (r *QuotaRepository) AddOutboundBytes(ctx context.Context, tenantID string, delta uint64) (*domain.Quota, error) {
	var q domain.Quota
	query := `
		UPDATE quotas SET
			used_outbound_bytes = CASE
				WHEN date_trunc('month', quota_period_start AT TIME ZONE 'UTC')
				     < date_trunc('month', now() AT TIME ZONE 'UTC')
				THEN $2
				ELSE used_outbound_bytes + $2
			END,
			quota_period_start = CASE
				WHEN date_trunc('month', quota_period_start AT TIME ZONE 'UTC')
				     < date_trunc('month', now() AT TIME ZONE 'UTC')
				THEN date_trunc('month', now() AT TIME ZONE 'UTC')
				ELSE quota_period_start
			END
		WHERE tenant_id = $1
		RETURNING tenant_id, max_deployments, max_apps, max_workers, max_memory_mb, max_outbound_mb, used_outbound_bytes, quota_period_start`
	err := r.db.GetContext(ctx, &q, query, tenantID, int64(delta))
	if err == sql.ErrNoRows {
		return nil, nil
	}
	return &q, err
}

func (r *QuotaRepository) Update(ctx context.Context, q *domain.Quota) error {
	query := `UPDATE quotas SET max_deployments = $2, max_apps = $3, max_workers = $4, max_memory_mb = $5, max_outbound_mb = $6 WHERE tenant_id = $1`
	_, err := r.db.ExecContext(ctx, query, q.TenantID, q.MaxDeployments, q.MaxApps, q.MaxWorkers, q.MaxMemoryMB, q.MaxOutboundMB)
	return err
}
