-- Custom-domain bindings for issue #83. One row per tenant-owned FQDN
-- bound to one of the tenant's apps. Tenant-facing CRUD lives at
-- /api/v1/apps/{appName}/domains*; the internal ingress poller reads
-- the same table via /api/internal/domains.
--
-- Column set is fixed by internal/repository/domain.go — every
-- INSERT/SELECT in that file enumerates these 8 columns. If you
-- add/remove a column, update both the migration and the repository.
--
-- 1. id — prefixed `dom_`; minted by the service layer (idempotency
--    keys aren't part of the v1 contract).
-- 2. tenant_id / app_name — the (tenant, app) this FQDN binds to.
--    No FK cascade from apps: an orphaned `domains` row whose
--    underlying app was deleted will keep authorizing TLS issuance
--    until v2 ships the cascade migration. The existing
--    TestInternal_TlsAllowed_OrphanedDomain_KnownGap test pins this
--    behaviour so the v2 fix has a clear pass/fail signal.
-- 3. fqdn — the tenant-owned hostname. Globally unique (UNIQUE
--    constraint). Lowercased on write; the service layer rejects
--    mixed-case before INSERT.
-- 4. status — pending (default at insert), active, failed. v1 only
--    ever persists `pending`; the active/failed states are reserved
--    for the v2 Caddy event hook (handler.UpdateDomainStatus).
-- 5. last_error — set when a v2 webhook flips status to `failed`.
--    Nullable; never written in v1.
-- 6. created_at — wall-clock at insert. Used as the default ORDER BY
--    for ListByApp so tenants see their bindings newest-first.
-- 7. verified_at — wall-clock when status last transitioned to
--    `active`. Nullable; reserved for v2.

CREATE TABLE domains (
    id          TEXT PRIMARY KEY,
    tenant_id   TEXT NOT NULL,
    app_name    TEXT NOT NULL,
    fqdn        TEXT NOT NULL UNIQUE,
    status      TEXT NOT NULL DEFAULT 'pending',
    last_error  TEXT,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    verified_at TIMESTAMPTZ
);

-- Hot path: ingress poller reads by (tenant_id, app_name) when
-- composing per-tenant route subsets (heartbeats.rs diff).
CREATE INDEX idx_domains_tenant_app ON domains(tenant_id, app_name);

-- Hot path: TlsAllowed handler is per-FQDN (Caddy's on_demand.ask
-- callback runs once per hostname during ACME issuance).
CREATE INDEX idx_domains_fqdn ON domains(fqdn);