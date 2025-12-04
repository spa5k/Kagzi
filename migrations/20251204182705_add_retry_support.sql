-- Add retry policy to workflow_runs
ALTER TABLE kagzi.workflow_runs
ADD COLUMN retry_policy JSONB;

-- Backfill existing rows to prevent NULL in application logic
UPDATE kagzi.workflow_runs
SET retry_policy = '{"maximum_attempts": 5, "initial_interval_ms": 1000, "backoff_coefficient": 2.0, "maximum_interval_ms": 60000}'::jsonb
WHERE retry_policy IS NULL;

-- Add retry scheduling to step_runs
ALTER TABLE kagzi.step_runs
ADD COLUMN retry_at TIMESTAMPTZ,
ADD COLUMN retry_policy JSONB;

-- Index for retry scheduling
CREATE INDEX idx_step_runs_retry
ON kagzi.step_runs (run_id, retry_at)
WHERE retry_at IS NOT NULL;
