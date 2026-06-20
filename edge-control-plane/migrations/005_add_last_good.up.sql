-- Add last_good_deployment_id column to active_deployments for issue #74
-- (CLI rollback UX). On a successful Activate, the previous deployment_id is
-- copied into this column; RollbackDeployment then swaps them back atomically.
-- Nullable so pre-existing rows (no history) read back as NULL and surface a
-- friendly 409 on rollback rather than crashing.
ALTER TABLE active_deployments
    ADD COLUMN last_good_deployment_id TEXT
        REFERENCES deployments(id);
