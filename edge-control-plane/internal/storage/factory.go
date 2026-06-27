package storage

import (
	"context"
	"fmt"
)

// BackendConfig is the storage-package-local view of the operator-facing
// artifact store configuration. Defined here (not imported from the
// `config` package) so this package has no upward dependency on
// `config` — the caller (cmd/api/main.go) translates from
// `config.StorageConfig` into `storage.BackendConfig` at wire-up.
//
// Every field is optional except as noted in the per-backend constructor
// docs; the factory selects the implementation from `ArtifactBackend`
// and validates the per-backend required fields up-front so a typo in
// the operator's config fails fast at startup, not on the first
// artifact request.
type BackendConfig struct {
	// ArtifactBackend selects the implementation. "" or "fs" → filesystem
	// (default); "s3" → S3ArtifactStore; "remote" → RemoteArtifactStore.
	ArtifactBackend string

	// ArtifactPath is the filesystem root for FSArtifactStore and the
	// cache dir for RemoteArtifactStore. Required when backend != "s3".
	ArtifactPath string

	// S3ArtifactStore fields. Ignored unless ArtifactBackend == "s3".
	S3Bucket    string
	S3Region    string
	S3Endpoint  string // optional, for minio/R2/LocalStack
	S3PathStyle bool   // true for minio; false for AWS
	S3KeyPrefix string // optional, e.g. "tenants/"

	// RemoteArtifactStore fields. Ignored unless ArtifactBackend == "remote".
	PeerControlPlaneURL           string
	PeerControlPlaneInternalToken string
}

// New selects and constructs the ArtifactStore for the configured
// backend. An empty `ArtifactBackend` defaults to "fs" so existing
// deployments need no config change. An unrecognized backend name
// returns an error so a typo in config fails at startup, not silently
// on first deploy.
//
// `ctx` is forwarded to backends that need it at construction time
// (today: only S3ArtifactStore, which uses it to load AWS config).
func New(ctx context.Context, cfg BackendConfig) (ArtifactStore, error) {
	backend := cfg.ArtifactBackend
	if backend == "" {
		backend = "fs"
	}
	switch backend {
	case "fs":
		return NewFSArtifactStore(cfg.ArtifactPath), nil
	case "s3":
		// NewS3ArtifactStore is added in Step 2. Until then, an
		// "s3" backend selection will surface a clear error.
		return NewS3ArtifactStore(ctx, cfg)
	case "remote":
		// NewRemoteArtifactStore is added in Step 3.
		return NewRemoteArtifactStore(cfg)
	default:
		return nil, fmt.Errorf("unknown artifact backend %q (want fs|s3|remote)", backend)
	}
}
