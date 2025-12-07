-- Rename business_id to external_id for user-provided identifiers
ALTER TABLE kagzi.workflow_runs RENAME COLUMN business_id TO external_id;

-- Rename idempotency_key to idempotency_suffix (used for scheduler-fired runs)
ALTER TABLE kagzi.workflow_runs RENAME COLUMN idempotency_key TO idempotency_suffix;

-- Drop legacy idempotency index
DROP INDEX IF EXISTS kagzi.idx_workflow_idempotency;

-- Enforce one active workflow per (namespace, external_id, suffix)
CREATE UNIQUE INDEX uq_active_workflow
ON kagzi.workflow_runs(namespace_id, external_id, COALESCE(idempotency_suffix, ''))
WHERE status NOT IN ('COMPLETED', 'FAILED', 'CANCELLED');

