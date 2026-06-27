package storage

import (
	"context"
	"fmt"
	"io"
)

// S3ArtifactStore persists WASM artifacts in an S3-compatible object
// store. Save is a PutObject, Open is a GetObject, Delete is a
// DeleteObject. Optional Endpoint + PathStyle fields configure
// minio/R2/LocalStack; the production path uses real AWS S3.
//
// Full implementation lands in Step 2 of the issue #127 plan. This
// stub returns a clear error today so an operator who selects
// `artifact_backend: "s3"` before the implementation lands gets a
// startup-time failure (not a silent per-deploy error).
type S3ArtifactStore struct {
	bucket    string
	region    string
	endpoint  string
	pathStyle bool
	keyPrefix string
}

// NewS3ArtifactStore validates the required S3 config fields and
// constructs the store. Returns an error if S3Bucket or S3Region is
// empty — the operator's config is incomplete and we'd rather fail at
// startup than on the first deploy.
//
// Full implementation (PutObject / GetObject / DeleteObject) lands in
// Step 2 of the issue #127 plan. Until then, calling Save / Open /
// Delete on the returned store returns an error.
func NewS3ArtifactStore(ctx context.Context, cfg BackendConfig) (*S3ArtifactStore, error) {
	if cfg.S3Bucket == "" {
		return nil, fmt.Errorf("S3ArtifactStore: S3Bucket is required")
	}
	if cfg.S3Region == "" {
		return nil, fmt.Errorf("S3ArtifactStore: S3Region is required")
	}
	return &S3ArtifactStore{
		bucket:    cfg.S3Bucket,
		region:    cfg.S3Region,
		endpoint:  cfg.S3Endpoint,
		pathStyle: cfg.S3PathStyle,
		keyPrefix: cfg.S3KeyPrefix,
	}, nil
}

// Save will PutObject once Step 2 lands. Until then, returns an error
// so a misconfigured `artifact_backend: "s3"` fails loudly.
func (s *S3ArtifactStore) Save(ctx context.Context, tenantID, appName, deploymentID string, r io.Reader) error {
	return fmt.Errorf("S3ArtifactStore.Save: not yet implemented (issue #127 step 2)")
}

// Open will GetObject once Step 2 lands.
func (s *S3ArtifactStore) Open(ctx context.Context, tenantID, appName, deploymentID string) (io.ReadCloser, error) {
	return nil, fmt.Errorf("S3ArtifactStore.Open: not yet implemented (issue #127 step 2)")
}

// Delete will DeleteObject once Step 2 lands.
func (s *S3ArtifactStore) Delete(ctx context.Context, tenantID, appName, deploymentID string) error {
	return fmt.Errorf("S3ArtifactStore.Delete: not yet implemented (issue #127 step 2)")
}
