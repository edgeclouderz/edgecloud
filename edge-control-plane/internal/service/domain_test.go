package service

import (
	"context"
	"errors"
	"testing"
	"time"

	"github.com/edgeclouderz/edge-cloud/edge-control-plane/internal/domain"
)

// mockDomainRepo implements DomainRepositoryInterface for testing.
// Each method delegates to a closure so individual tests can wire only
// the methods they exercise.
type mockDomainRepo struct {
	createFn       func(ctx context.Context, d *domain.Domain) error
	getByIDFn      func(ctx context.Context, id string) (*domain.Domain, error)
	getByFQDNFn    func(ctx context.Context, fqdn string) (*domain.Domain, error)
	listByAppFn    func(ctx context.Context, tenantID, appName string) ([]domain.Domain, error)
	countByAppFn   func(ctx context.Context, tenantID, appName string) (int, error)
	listAllFn      func(ctx context.Context) ([]domain.Domain, error)
	atomicDeleteFn func(ctx context.Context, tenantID, appName, fqdn string) (bool, error)
	updateStatusFn func(ctx context.Context, id string, status domain.DomainStatus, lastError *string) error
}

func (m *mockDomainRepo) Create(ctx context.Context, d *domain.Domain) error {
	if m.createFn == nil {
		return nil
	}
	return m.createFn(ctx, d)
}
func (m *mockDomainRepo) GetByID(ctx context.Context, id string) (*domain.Domain, error) {
	if m.getByIDFn == nil {
		return nil, nil
	}
	return m.getByIDFn(ctx, id)
}
func (m *mockDomainRepo) GetByFQDN(ctx context.Context, fqdn string) (*domain.Domain, error) {
	if m.getByFQDNFn == nil {
		return nil, nil
	}
	return m.getByFQDNFn(ctx, fqdn)
}
func (m *mockDomainRepo) ListByApp(ctx context.Context, tenantID, appName string) ([]domain.Domain, error) {
	if m.listByAppFn == nil {
		return nil, nil
	}
	return m.listByAppFn(ctx, tenantID, appName)
}
func (m *mockDomainRepo) CountByApp(ctx context.Context, tenantID, appName string) (int, error) {
	if m.countByAppFn == nil {
		return 0, nil
	}
	return m.countByAppFn(ctx, tenantID, appName)
}
func (m *mockDomainRepo) ListAll(ctx context.Context) ([]domain.Domain, error) {
	if m.listAllFn == nil {
		return nil, nil
	}
	return m.listAllFn(ctx)
}
func (m *mockDomainRepo) AtomicDelete(ctx context.Context, tenantID, appName, fqdn string) (bool, error) {
	if m.atomicDeleteFn == nil {
		return true, nil
	}
	return m.atomicDeleteFn(ctx, tenantID, appName, fqdn)
}
func (m *mockDomainRepo) UpdateStatus(ctx context.Context, id string, status domain.DomainStatus, lastError *string) error {
	if m.updateStatusFn == nil {
		return nil
	}
	return m.updateStatusFn(ctx, id, status, lastError)
}

// mockAppLookupForDomain implements appLookupForDomain for testing.
type mockAppLookupForDomain struct {
	getFn func(ctx context.Context, tenantID, appName string) (*domain.App, error)
}

func (m *mockAppLookupForDomain) Get(ctx context.Context, tenantID, appName string) (*domain.App, error) {
	if m.getFn == nil {
		return &domain.App{}, nil
	}
	return m.getFn(ctx, tenantID, appName)
}

func domainSvcForTest(repo DomainRepositoryInterface, appSvc appLookupForDomain) *DomainService {
	return &DomainService{domainRepo: repo, appSvc: appSvc}
}

// TestIsValidFQDN covers the FQDN shape gate. The full matrix is wide
// but every entry here is a regression the production handler depends
// on; if the regex loosens or tightens by accident, this table fails
// before the change ships.
func TestIsValidFQDN(t *testing.T) {
	cases := []struct {
		fqdn string
		want bool
		why  string
	}{
		{"api.acme.com", true, "standard FQDN"},
		{"a.b.c.d.e.f", true, "many short labels"},
		{"single", true, "single-label FQDN"},
		{"x", true, "single character"},
		{"api-v1.acme.com", true, "hyphens in labels"},
		{"123.example.com", true, "leading digit label"},
		{"UPPER.example.com", false, "uppercase rejected (DNS is case-insensitive, ops want consistency)"},
		{"-leading.example.com", false, "leading hyphen rejected"},
		{"trailing-.example.com", false, "trailing hyphen rejected"},
		{"foo..bar.com", false, "empty label rejected"},
		{".foo.com", false, "leading dot rejected"},
		{"foo.com.", false, "trailing dot rejected (would need explicit allow)"},
		{"api.example.com " + string(make([]byte, 200)), false, "over 253 chars rejected"},
		{"api.edgecloud.dev", false, "platform suffix rejected"},
		{"myapp.svc.edgecloud.dev", false, "any .edgecloud.dev suffix rejected"},
		{"*.example.com", false, "wildcard rejected (DNS-01 out of scope)"},
		{"api.example.com:8081", false, "port suffix rejected"},
		{"api/example.com", false, "slash rejected"},
		{"", false, "empty rejected"},
		{"foo bar.com", false, "whitespace rejected"},
	}
	for _, c := range cases {
		t.Run(c.fqdn, func(t *testing.T) {
			got := IsValidFQDN(c.fqdn)
			if got != c.want {
				t.Errorf("IsValidFQDN(%q) = %v, want %v (%s)", c.fqdn, got, c.want, c.why)
			}
		})
	}
}

func TestDomainService_AddDomain_RejectsInvalidFQDN(t *testing.T) {
	appSvc := &mockAppLookupForDomain{
		getFn: func(ctx context.Context, tenantID, appName string) (*domain.App, error) {
			return &domain.App{ID: "a_x", TenantID: tenantID, Name: appName}, nil
		},
	}
	svc := domainSvcForTest(&mockDomainRepo{}, appSvc)
	_, err := svc.AddDomain(context.Background(), "t_a", "api", "INVALID.example.com")
	if !errors.Is(err, ErrInvalidFQDN) {
		t.Fatalf("AddDomain(uppercase) = %v, want ErrInvalidFQDN", err)
	}
}

func TestDomainService_AddDomain_RejectsEdgecloudDevSuffix(t *testing.T) {
	appSvc := &mockAppLookupForDomain{
		getFn: func(ctx context.Context, tenantID, appName string) (*domain.App, error) {
			return &domain.App{ID: "a_x", TenantID: tenantID, Name: appName}, nil
		},
	}
	svc := domainSvcForTest(&mockDomainRepo{}, appSvc)
	_, err := svc.AddDomain(context.Background(), "t_a", "api", "api.edgecloud.dev")
	if !errors.Is(err, ErrInvalidFQDN) {
		t.Fatalf("AddDomain(.edgecloud.dev) = %v, want ErrInvalidFQDN", err)
	}
}

func TestDomainService_AddDomain_RejectsUnknownApp(t *testing.T) {
	appSvc := &mockAppLookupForDomain{
		getFn: func(ctx context.Context, tenantID, appName string) (*domain.App, error) {
			return nil, nil // no app
		},
	}
	svc := domainSvcForTest(&mockDomainRepo{}, appSvc)
	_, err := svc.AddDomain(context.Background(), "t_a", "api", "api.acme.com")
	if !errors.Is(err, ErrAppNotFound) {
		t.Fatalf("AddDomain(no app) = %v, want ErrAppNotFound", err)
	}
}

func TestDomainService_AddDomain_RejectsQuotaExceeded(t *testing.T) {
	appSvc := &mockAppLookupForDomain{
		getFn: func(ctx context.Context, tenantID, appName string) (*domain.App, error) {
			return &domain.App{ID: "a_x", TenantID: tenantID, Name: appName}, nil
		},
	}
	repo := &mockDomainRepo{
		countByAppFn: func(ctx context.Context, tenantID, appName string) (int, error) {
			return MaxDomainsPerApp, nil // already at cap
		},
	}
	svc := domainSvcForTest(repo, appSvc)
	_, err := svc.AddDomain(context.Background(), "t_a", "api", "api.acme.com")
	if !errors.Is(err, ErrDomainQuotaExceeded) {
		t.Fatalf("AddDomain(at cap) = %v, want ErrDomainQuotaExceeded", err)
	}
}

func TestDomainService_AddDomain_HappyPath(t *testing.T) {
	appSvc := &mockAppLookupForDomain{
		getFn: func(ctx context.Context, tenantID, appName string) (*domain.App, error) {
			return &domain.App{ID: "a_x", TenantID: tenantID, Name: appName}, nil
		},
	}
	var captured *domain.Domain
	repo := &mockDomainRepo{
		countByAppFn: func(ctx context.Context, tenantID, appName string) (int, error) { return 0, nil },
		createFn: func(ctx context.Context, d *domain.Domain) error {
			captured = d
			return nil
		},
	}
	svc := domainSvcForTest(repo, appSvc)
	d, err := svc.AddDomain(context.Background(), "t_a", "api", "api.acme.com")
	if err != nil {
		t.Fatalf("AddDomain: %v", err)
	}
	if captured == nil {
		t.Fatalf("Create was not called")
	}
	if captured.Status != domain.DomainStatusPending {
		t.Errorf("created status = %q, want pending", captured.Status)
	}
	if captured.FQDN != "api.acme.com" {
		t.Errorf("created fqdn = %q, want api.acme.com", captured.FQDN)
	}
	if d.ID == "" || d.ID[:4] != "dom_" {
		t.Errorf("created id = %q, want dom_<uuid>", d.ID)
	}
	if d.CreatedAt.IsZero() {
		t.Errorf("created_at not set")
	}
}

// TestIsTlsAllowed covers the three answers Caddy's ask URL can return:
// "yes, issue a cert" (pending or active), "no, refuse" (not found or
// failed). The ingress's renderer is keyed off this answer.
func TestDomainService_IsTlsAllowed(t *testing.T) {
	cases := []struct {
		name   string
		status domain.DomainStatus
		want   bool
	}{
		{"pending authorizes", domain.DomainStatusPending, true},
		{"active authorizes", domain.DomainStatusActive, true},
		{"failed does not authorize", domain.DomainStatusFailed, false},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			repo := &mockDomainRepo{
				getByFQDNFn: func(ctx context.Context, fqdn string) (*domain.Domain, error) {
					return &domain.Domain{Status: c.status}, nil
				},
			}
			svc := domainSvcForTest(repo, &mockAppLookupForDomain{})
			got, err := svc.IsTlsAllowed(context.Background(), "api.acme.com")
			if err != nil {
				t.Fatalf("IsTlsAllowed: %v", err)
			}
			if got != c.want {
				t.Errorf("IsTlsAllowed(status=%s) = %v, want %v", c.status, got, c.want)
			}
		})
	}
}

func TestDomainService_IsTlsAllowed_UnknownFQDN(t *testing.T) {
	repo := &mockDomainRepo{
		getByFQDNFn: func(ctx context.Context, fqdn string) (*domain.Domain, error) {
			return nil, nil // not found
		},
	}
	svc := domainSvcForTest(repo, &mockAppLookupForDomain{})
	got, err := svc.IsTlsAllowed(context.Background(), "api.acme.com")
	if err != nil {
		t.Fatalf("IsTlsAllowed: %v", err)
	}
	if got {
		t.Errorf("IsTlsAllowed(unknown) = true, want false")
	}
}

// TestDomainService_GetDomain_TenantScope pins that GetDomain refuses
// to return a row when the request's (tenant, app) does not match the
// stored row — even if the FQDN is the same. Two tenants could in theory
// bind the same FQDN at different times (e.g. after a delete); the
// service-level guard ensures the wrong tenant never observes the row.
func TestDomainService_GetDomain_TenantScope(t *testing.T) {
	repo := &mockDomainRepo{
		getByFQDNFn: func(ctx context.Context, fqdn string) (*domain.Domain, error) {
			return &domain.Domain{TenantID: "t_other", AppName: "api", FQDN: fqdn}, nil
		},
	}
	svc := domainSvcForTest(repo, &mockAppLookupForDomain{})
	_, err := svc.GetDomain(context.Background(), "t_a", "api", "api.acme.com")
	if !errors.Is(err, ErrDomainNotFound) {
		t.Fatalf("GetDomain(wrong tenant) = %v, want ErrDomainNotFound", err)
	}
}

func TestDomainService_RemoveDomain_NotFoundReturnsSentinel(t *testing.T) {
	repo := &mockDomainRepo{
		atomicDeleteFn: func(ctx context.Context, tenantID, appName, fqdn string) (bool, error) {
			return false, nil // 0 rows affected
		},
	}
	svc := domainSvcForTest(repo, &mockAppLookupForDomain{})
	err := svc.RemoveDomain(context.Background(), "t_a", "api", "api.acme.com")
	if !errors.Is(err, ErrDomainNotFound) {
		t.Fatalf("RemoveDomain(no row) = %v, want ErrDomainNotFound", err)
	}
}

func TestDomainService_RemoveDomain_HappyPath(t *testing.T) {
	called := false
	repo := &mockDomainRepo{
		atomicDeleteFn: func(ctx context.Context, tenantID, appName, fqdn string) (bool, error) {
			called = true
			return true, nil
		},
	}
	svc := domainSvcForTest(repo, &mockAppLookupForDomain{})
	if err := svc.RemoveDomain(context.Background(), "t_a", "api", "api.acme.com"); err != nil {
		t.Fatalf("RemoveDomain: %v", err)
	}
	if !called {
		t.Errorf("AtomicDelete was not called")
	}
}

// TestDomainService_ListAllDomains_ReturnsRows verifies the ingress
// poll endpoint returns every domain row (across tenants). The
// ingress's 30s tick uses this; the JSON shape must be a flat array.
func TestDomainService_ListAllDomains_ReturnsRows(t *testing.T) {
	now := time.Now()
	repo := &mockDomainRepo{
		listAllFn: func(ctx context.Context) ([]domain.Domain, error) {
			return []domain.Domain{
				{ID: "dom_1", TenantID: "t_a", AppName: "api", FQDN: "api.acme.com", Status: domain.DomainStatusPending, CreatedAt: now},
				{ID: "dom_2", TenantID: "t_b", AppName: "api", FQDN: "web.acme.com", Status: domain.DomainStatusActive, CreatedAt: now},
			}, nil
		},
	}
	svc := domainSvcForTest(repo, &mockAppLookupForDomain{})
	got, err := svc.ListAllDomains(context.Background())
	if err != nil {
		t.Fatalf("ListAllDomains: %v", err)
	}
	if len(got) != 2 {
		t.Fatalf("len(domains) = %d, want 2", len(got))
	}
}
