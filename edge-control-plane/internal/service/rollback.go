package service

import (
	"context"
	"errors"
	"fmt"
	"log"
	"os"
)

// rollbackArtifactSave performs the compensating-delete sequence when an
// artifact save fails after a deployment row has already been inserted.
// It removes the row from the database AND any partial blob the
// non-atomic Save may have left on disk, then returns the wrapped save
// error to surface to the caller.
//
// The function is a free function (not a method) because it only needs
// the two store contracts and the deployment id — no service state.
// Centralizing the rollback here keeps the three call sites (Migrate,
// MigrateTree, Deploy) in lockstep: a future fix to one site updates
// the other two at the same time.
//
// `ArtifactStore.Save` is non-atomic (os.Create + io.Copy, no
// temp-rename pattern), so a mid-write io.Copy failure can leave bytes
// on disk. Without the Delete call below, that partial blob would
// leak until the next successful Save overwrites the same path.
//
// The Delete errors are logged (not swallowed) and the original save
// error is returned. Errors.Is(err, os.ErrNotExist) is treated as
// success for the artifact delete: it just means Save failed before
// the file was ever created, so there's nothing to clean up.
func rollbackArtifactSave(
	ctx context.Context,
	repo DeploymentRepoInterface,
	store ArtifactStoreInterface,
	tenantID, appName, depID string,
	saveErr error,
) error {
	if delErr := repo.DeleteByID(ctx, depID); delErr != nil {
		log.Printf("rollback DeleteByID failed after artifact save error: deployment_id=%s error=%v", depID, delErr)
	}
	if delErr := store.Delete(tenantID, appName, depID); delErr != nil && !errors.Is(delErr, os.ErrNotExist) {
		log.Printf("rollback artifact.Delete failed after artifact save error: deployment_id=%s error=%v", depID, delErr)
	}
	return fmt.Errorf("saving artifact: %w", saveErr)
}