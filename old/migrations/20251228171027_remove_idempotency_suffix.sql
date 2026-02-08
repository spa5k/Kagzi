-- Remove idempotency_suffix from unique index
DROP INDEX IF EXISTS kagzi.uq_active_workflow;

-- Recreate without idempotency_suffix
CREATE UNIQUE INDEX IF NOT EXISTS uq_active_workflow
    ON kagzi.workflow_runs (namespace_id, external_id)
    WHERE status NOT IN ('COMPLETED', 'FAILED', 'CANCELLED');

-- Drop column
ALTER TABLE kagzi.workflow_runs DROP COLUMN IF EXISTS idempotency_suffix;

