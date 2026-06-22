ALTER TABLE quotas ADD COLUMN quota_period_start TIMESTAMPTZ NOT NULL DEFAULT date_trunc('month', now() AT TIME ZONE 'UTC');
