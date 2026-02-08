-- Phase 4: Remove step-level retry_at column
-- Step retries are now handled via workflow replay with memoization

DROP INDEX IF EXISTS kagzi.idx_step_runs_pending_retry;
DROP INDEX IF EXISTS kagzi.idx_step_runs_retry;
ALTER TABLE kagzi.step_runs DROP COLUMN IF EXISTS retry_at;
