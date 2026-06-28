package service

import (
	"errors"
	"log"
	"os"
)

// rollbackArtifactSave performs best-effort cleanup of any partial
// blob `ArtifactStore.Save` may have left on disk when it failed
// after the deployment row was already written (or, in the tx path,
// after the row was inserted into the open transaction). The row
// itself is no longer this function's concern: the tx path rolls it
// back atomically, and the no-tx fallback in Deploy handles its own
// compensating DeleteByID. This helper only deals with the file.
//
// `Save` is now atomic via the temp-rename pattern, so a failed save
// never leaves bytes at the final path. The Delete below is
// defense-in-depth for a future regression — silent data loss
// otherwise.
//
// The function returns saveErr UNWRAPPED (no `fmt.Errorf` wrap) so
// the caller can wrap with the appropriate sentinel
// (ErrMigrationFailed / ErrMigrateTreeFailed) before surfacing to
// the HTTP layer. The earlier hard-coded "saving artifact" wrap
// broke `isClientMigrationError`'s sentinel match on disk-full
// errors and made the handler return 500 instead of 422.
func rollbackArtifactSave(
	store ArtifactStoreInterface,
	tenantID, appName, depID string,
	saveErr error,
) error {
	if delErr := store.Delete(tenantID, appName, depID); delErr != nil && !errors.Is(delErr, os.ErrNotExist) {
		log.Printf("rollback artifact.Delete failed after artifact save error: deployment_id=%s error=%v", depID, delErr)
	}
	return saveErr
}