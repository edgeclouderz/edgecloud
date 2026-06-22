package domain

import (
	"time"

	"github.com/lib/pq"
)

// Deployment represents a deployed Wasm artifact.
type Deployment struct {
	ID       string `db:"id"`
	TenantID string `db:"tenant_id"`
	AppName  string `db:"app_name"`
	Status   string `db:"status"`
	Hash     string `db:"hash"` // SHA-256 of Wasm payload
	// Regions is the list of regions this deployment is replicated to.
	// The activate path loops over this list and publishes one
	// `TaskMessage` per region to `edgecloud.tasks.<region>`. An empty
	// slice (e.g. for rows created before migration 008) means
	// "use the control plane's default region" — the service layer
	// resolves the fallback. See `service.ActivateDeployment`.
	//
	// Typed as pq.StringArray (which is `[]string` underneath) so the
	// `TEXT[]` column scans correctly via lib/pq's Scanner — a bare
	// `[]string` does NOT implement `sql.Scanner` and would fail on
	// SELECT. The JSON wire format is unchanged because
	// pq.StringArray marshals identically to []string.
	//
	// No `omitempty`: an empty slice serializes as `[]`, which is
	// more useful for clients than `null` and matches the codebase
	// convention of not using `omitempty` on domain structs.
	Regions   pq.StringArray `db:"regions" json:"regions"`
	CreatedAt time.Time      `db:"created_at"`
	// AutoRollbackEnabled is the tenant opt-in set by
	// `edge deploy --auto-rollback`. At activate time this flag is
	// copied onto the active_deployments row; it controls whether
	// the worker-driven auto-rollback (handler.AutoRollback) and the
	// heartbeat-driven stability window (service.worker.evaluateStability)
	// are allowed to mutate last_good_deployment_id for this app.
	// Defaults to false on the wire (legacy deployments pre-migration-009
	// are not affected). Stored on the deployments row too so operators
	// can audit "which deployments opted in" via the list endpoint.
	AutoRollbackEnabled bool `db:"auto_rollback_enabled" json:"auto_rollback_enabled"`
}

// Deployment status constants.
const (
	StatusDeployed = "deployed"
	StatusActive   = "active"
	StatusFailed   = "failed"
	StatusMigrated = "migrated"
)

// ActiveDeployment maps an app name to its active deployment for a tenant.
//
// LastGoodDeploymentID is the prior deployment that was active before the
// most recent Activate. Used by RollbackDeployment to swap back to it
// without requiring the tenant to remember the id. Nullable: pre-existing
// rows (no history) read back as nil; rollback on such a row returns 409.
type ActiveDeployment struct {
	TenantID             string  `db:"tenant_id"`
	AppName              string  `db:"app_name"`
	DeploymentID         string  `db:"deployment_id"`
	LastGoodDeploymentID *string `db:"last_good_deployment_id"`
	// AutoRollbackEnabled mirrors the flag from the deployments
	// row, copied at activate time. Read by the worker-driven
	// auto-rollback endpoint and by the heartbeat-driven stability
	// window. Defaults to false on disk (migration 009).
	AutoRollbackEnabled bool `db:"auto_rollback_enabled"`
	// StableSince is the first-heartbeat timestamp for the
	// currently-active deployment. NULL means "not yet observed
	// running" or "rolled back; clock reset". The heartbeat
	// handler sets this to NOW() the first time it sees
	// status="running" for this active row; the stability window
	// promotes deployment_id → last_good_deployment_id once
	// stable_since is older than STABLE_WINDOW_SECONDS. Reset to
	// NULL on every activate / rollback / auto-rollback (see
	// service.ActivateDeployment / RollbackDeployment and
	// repository.ResetStableSinceForRollback).
	StableSince *time.Time `db:"stable_since"`
}

// AppEnv stores environment variables for an app.
type AppEnv struct {
	TenantID string `db:"tenant_id"`
	AppName  string `db:"app_name"`
	EnvKey   string `db:"env_key"`
	EnvValue string `db:"env_value"`
}
