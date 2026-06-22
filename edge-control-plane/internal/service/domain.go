package service

import (
	"context"
	"errors"
	"fmt"
	"regexp"
	"strings"
	"time"

	"github.com/edgeclouderz/edge-cloud/edge-control-plane/internal/domain"
	"github.com/edgeclouderz/edge-cloud/edge-control-plane/internal/repository"
	"github.com/google/uuid"
)

// MaxDomainsPerApp caps the number of custom domains a single app can
// claim. Defensive ceiling against abuse; realistic tenants want ≤5.
// Operators can raise this constant if needed. Mirrors the pattern set
// by `MaxRegionsPerDeployment` (issue #82).
const MaxDomainsPerApp = 50

// edgecloudDevSuffix is the wildcard host suffix the platform manages
// (`<tenant>-<app>.edgecloud.dev`). Custom domains cannot share this
// suffix because they'd collide with the synthetic hostname namespace;
// rejecting it here is defense-in-depth on top of the UNIQUE constraint
// on `domains.fqdn`.
const edgecloudDevSuffix = ".edgecloud.dev"

// fqdnPattern is the RFC 1035-ish shape we accept. Each label is
// 1-63 chars, `[a-z0-9-]`, no leading/trailing hyphen, lowercase only
// (DNS is case-insensitive but case-sensitive operators cause
// `tls-allowed` lookup misses and `edge domains check` confusion). We
// reject wildcards (`*`) because v1 only does single-FQDN HTTP-01 ACME;
// wildcard support requires DNS-01 and is deferred to v2.
//
// The regex is intentionally NOT anchored to a max total length — that's
// a separate length check below so the error message can name the
// offending value.
var fqdnPattern = regexp.MustCompile(`^[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?(\.[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?)*$`)

// IsValidFQDN returns true if the FQDN shape is acceptable. Rejects:
//   - empty strings,
//   - strings longer than 253 chars (DNS hard limit),
//   - any character outside `[a-z0-9.-]` (uppercase, whitespace, `*`,
//     `..`, leading/trailing dot, etc.),
//   - labels starting or ending with a hyphen,
//   - labels longer than 63 chars (DNS hard limit),
//   - FQDNs ending in `.edgecloud.dev` (platform-managed host).
//
// Modeled on `IsValidAppName` / `IsValidRegion`. The service layer
// rejects invalid FQDNs before they reach the DB or the ingress poller.
func IsValidFQDN(fqdn string) bool {
	if fqdn == "" || len(fqdn) > 253 {
		return false
	}
	if strings.HasSuffix(fqdn, edgecloudDevSuffix) {
		return false
	}
	if strings.Contains(fqdn, "*") {
		return false
	}
	return fqdnPattern.MatchString(fqdn)
}

// Sentinel errors.
//
// The handler matches these via errors.Is and maps them to HTTP status
// codes (400, 404, 429 respectively).
//
// `ErrAppNotFound` is defined in app.go (it's the cross-service "app
// does not exist" sentinel); we wrap it via `%w` so handlers can match.
var (
	ErrInvalidFQDN         = errors.New("invalid fqdn")
	ErrDomainNotFound      = errors.New("domain not found")
	ErrDomainQuotaExceeded = errors.New("too many domains for this app")
)

// appLookupForDomain is the narrow contract DomainService needs from
// AppService. Kept as an interface so handler/service tests can mock
// just the (tenant, app) existence check without standing up the full
// AppService + AppRepository + QuotaRepository graph.
type appLookupForDomain interface {
	Get(ctx context.Context, tenantID, appName string) (*domain.App, error)
}

// DomainRepositoryInterface is the narrow contract DomainService needs
// from DomainRepository. Mirrors the pattern in worker.go / app.go.
type DomainRepositoryInterface interface {
	Create(ctx context.Context, d *domain.Domain) error
	GetByID(ctx context.Context, id string) (*domain.Domain, error)
	GetByFQDN(ctx context.Context, fqdn string) (*domain.Domain, error)
	ListByApp(ctx context.Context, tenantID, appName string) ([]domain.Domain, error)
	CountByApp(ctx context.Context, tenantID, appName string) (int, error)
	ListAll(ctx context.Context) ([]domain.Domain, error)
	AtomicDelete(ctx context.Context, tenantID, appName, fqdn string) (bool, error)
	UpdateStatus(ctx context.Context, id string, status domain.DomainStatus, lastError *string) error
}

// DomainService handles custom-domain business logic.
type DomainService struct {
	domainRepo DomainRepositoryInterface
	appSvc     appLookupForDomain
}

// NewDomainService creates a new DomainService. The appSvc dependency is
// required: AddDomain refuses to bind an FQDN to a (tenant, app) pair
// that does not already exist (auto-creating an app on domain-add would
// surprise tenants with stray `apps` rows).
func NewDomainService(domainRepo *repository.DomainRepository, appSvc appLookupForDomain) *DomainService {
	return &DomainService{
		domainRepo: domainRepo,
		appSvc:     appSvc,
	}
}

// AddDomain validates the FQDN shape, ensures the (tenant, app) exists,
// enforces the per-app quota, and inserts the row in `pending` state.
// The ingress's 30s poller picks up the new row on its next tick.
//
// Returns ErrInvalidFQDN, ErrDomainQuotaExceeded, or an unwrapped DB
// error. Callers (handlers, tests) match via errors.Is.
func (s *DomainService) AddDomain(ctx context.Context, tenantID, appName, fqdn string) (*domain.Domain, error) {
	if !IsValidFQDN(fqdn) {
		return nil, fmt.Errorf("%w %q: must be RFC-1035 shape, lowercase, ≤253 chars, no wildcard, no .edgecloud.dev suffix", ErrInvalidFQDN, fqdn)
	}

	app, err := s.appSvc.Get(ctx, tenantID, appName)
	if err != nil {
		return nil, fmt.Errorf("looking up app: %w", err)
	}
	if app == nil {
		return nil, fmt.Errorf("%w: %s", ErrAppNotFound, appName)
	}

	count, err := s.domainRepo.CountByApp(ctx, tenantID, appName)
	if err != nil {
		return nil, fmt.Errorf("counting domains: %w", err)
	}
	if count >= MaxDomainsPerApp {
		return nil, fmt.Errorf("%w: %d (max %d)", ErrDomainQuotaExceeded, count, MaxDomainsPerApp)
	}

	d := &domain.Domain{
		ID:        "dom_" + uuid.New().String(),
		TenantID:  tenantID,
		AppName:   appName,
		FQDN:      fqdn,
		Status:    domain.DomainStatusPending,
		CreatedAt: time.Now(),
	}
	if err := s.domainRepo.Create(ctx, d); err != nil {
		return nil, fmt.Errorf("creating domain: %w", err)
	}
	return d, nil
}

// ListDomains returns all domains for a (tenant, app). Returns an empty
// slice (not nil, not an error) when the app has no domains — matches
// the existing pattern in DeploymentRepository.ListByApp.
func (s *DomainService) ListDomains(ctx context.Context, tenantID, appName string) ([]domain.Domain, error) {
	return s.domainRepo.ListByApp(ctx, tenantID, appName)
}

// GetDomain returns a single domain by (tenant, app, fqdn). Returns
// (nil, ErrDomainNotFound) when no row matches; the handler maps that
// to 404.
func (s *DomainService) GetDomain(ctx context.Context, tenantID, appName, fqdn string) (*domain.Domain, error) {
	d, err := s.domainRepo.GetByFQDN(ctx, fqdn)
	if err != nil {
		return nil, fmt.Errorf("looking up domain: %w", err)
	}
	if d == nil || d.TenantID != tenantID || d.AppName != appName {
		return nil, ErrDomainNotFound
	}
	return d, nil
}

// RemoveDomain deletes the row matching (tenant, app, fqdn). Returns
// ErrDomainNotFound when no row matched (the row may have been deleted
// by a concurrent request or the tenant is targeting the wrong (app,
// fqdn) pair). The 30s poller on the ingress will pick up the deletion
// on its next tick and drop the FQDN from its routing table.
func (s *DomainService) RemoveDomain(ctx context.Context, tenantID, appName, fqdn string) error {
	deleted, err := s.domainRepo.AtomicDelete(ctx, tenantID, appName, fqdn)
	if err != nil {
		return fmt.Errorf("deleting domain: %w", err)
	}
	if !deleted {
		return ErrDomainNotFound
	}
	return nil
}

// IsTlsAllowed answers Caddy's `on_demand.ask` query: should I issue a
// cert for this FQDN? Returns true iff a row exists for the FQDN in
// either `pending` or `active` state. `failed` rows do NOT authorize
// issuance (a previous ACME failure means re-trying the same cert
// would just fail again — the operator needs to fix the upstream
// issue first).
//
// Known gap: this does NOT check that the underlying (tenant, app)
// still exists. If the tenant is deleted, the FK cascade removes the
// domain row and we correctly return false. But if the *app* is
// deleted (no FK cascade), the domain row survives and we incorrectly
// return true. This is pinned by a handler test; the fix requires a
// composite FK on (tenant_id, app_name) → apps(tenant_id, name),
// deferred to a follow-up that also adds the missing unique index.
func (s *DomainService) IsTlsAllowed(ctx context.Context, fqdn string) (bool, error) {
	d, err := s.domainRepo.GetByFQDN(ctx, fqdn)
	if err != nil {
		return false, fmt.Errorf("looking up fqdn: %w", err)
	}
	if d == nil {
		return false, nil
	}
	return d.Status == domain.DomainStatusPending || d.Status == domain.DomainStatusActive, nil
}

// ListAllDomains returns every domain row across all tenants. Used by
// the ingress's `GET /api/internal/domains` poll endpoint. JWT-protected
// at the handler layer; the service trusts its caller.
func (s *DomainService) ListAllDomains(ctx context.Context) ([]domain.Domain, error) {
	return s.domainRepo.ListAll(ctx)
}

// GetDomainByID returns a domain by its primary key. Used by the v2
// Caddy event hook (which only sees the row id). Returns (nil, nil)
// when no row matches — the handler maps that to 404.
func (s *DomainService) GetDomainByID(ctx context.Context, id string) (*domain.Domain, error) {
	return s.domainRepo.GetByID(ctx, id)
}

// UpdateStatus updates the status (and optionally last_error) of a
// domain row. Used by the v2 Caddy event hook via
// `POST /api/internal/domains/{id}/status`. v1 has no callers.
func (s *DomainService) UpdateStatus(ctx context.Context, id string, status domain.DomainStatus, lastError *string) error {
	if err := s.domainRepo.UpdateStatus(ctx, id, status, lastError); err != nil {
		return fmt.Errorf("updating domain status: %w", err)
	}
	return nil
}
