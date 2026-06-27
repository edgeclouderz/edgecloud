package storage

import (
	"context"
	"fmt"
	"io"
)

// RemoteArtifactStore is a pull-through cache: a local FS cache in
// front of a peer control-plane that serves artifacts over HTTPS
// using the `X-Internal-Token` shared-secret auth. On Open, if the
// local cache has the blob, return it; if not, fetch from the peer
// CP, write to the cache (atomic rename), then return.
//
// The local CP acts as a CDN edge node — a worker in region B hits
// its own CP first, and only on a cold cache does the request go
// across regions to the originating CP. v1 keeps Save / Delete
// local-only; pre-warming peers is a follow-up issue.
//
// Full implementation lands in Step 3 of the issue #127 plan. This
// stub returns a clear error today so an operator who selects
// `artifact_backend: "remote"` before the implementation lands gets
// a startup-time failure (not a silent per-deploy error).
type RemoteArtifactStore struct {
	cache     *FSArtifactStore
	peerURL   string
	peerToken string
}

// NewRemoteArtifactStore validates the required peer config fields
// (URL + token + cache dir) and constructs the store. Fail-closed:
// empty peerURL or empty peerToken returns an error so a misconfigured
// peer can't silently fall back to an unauthenticated GET.
//
// Full implementation lands in Step 3 of the issue #127 plan.
func NewRemoteArtifactStore(cfg BackendConfig) (*RemoteArtifactStore, error) {
	if cfg.PeerControlPlaneURL == "" {
		return nil, fmt.Errorf("RemoteArtifactStore: PeerControlPlaneURL is required")
	}
	if cfg.PeerControlPlaneInternalToken == "" {
		return nil, fmt.Errorf("RemoteArtifactStore: PeerControlPlaneInternalToken is required")
	}
	if cfg.ArtifactPath == "" {
		return nil, fmt.Errorf("RemoteArtifactStore: ArtifactPath is required (local cache dir)")
	}
	return &RemoteArtifactStore{
		cache:     NewFSArtifactStore(cfg.ArtifactPath),
		peerURL:   cfg.PeerControlPlaneURL,
		peerToken: cfg.PeerControlPlaneInternalToken,
	}, nil
}

// Save writes only to the local cache (v1). Pre-warming peers is a
// follow-up issue; for now the peer CP pulls on first miss.
func (s *RemoteArtifactStore) Save(ctx context.Context, tenantID, appName, deploymentID string, r io.Reader) error {
	return s.cache.Save(ctx, tenantID, appName, deploymentID, r)
}

// Open will check the local cache and fall back to a peer CP GET
// once Step 3 lands. Until then, returns an error.
func (s *RemoteArtifactStore) Open(ctx context.Context, tenantID, appName, deploymentID string) (io.ReadCloser, error) {
	return nil, fmt.Errorf("RemoteArtifactStore.Open: not yet implemented (issue #127 step 3)")
}

// Delete removes the local cache entry only. Cross-CP GC is a
// separate concern.
func (s *RemoteArtifactStore) Delete(ctx context.Context, tenantID, appName, deploymentID string) error {
	return s.cache.Delete(ctx, tenantID, appName, deploymentID)
}
