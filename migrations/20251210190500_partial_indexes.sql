-- NOTE: For production deployments run these with CONCURRENTLY outside a transaction to avoid table locks:
--   CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_workflow_runs_claimable
--   ON kagzi.workflow_runs (task_queue, namespace_id, COALESCE(wake_up_at, created_at))
--   WHERE status IN ('PENDING', 'SLEEPING');
--   CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_step_runs_pending_retry
--   ON kagzi.step_runs (retry_at)
--   WHERE status = 'PENDING' AND retry_at IS NOT NULL;
-- For dev/test environments (transactional migrations), use non-concurrent indexes below.
CREATE INDEX IF NOT EXISTS idx_workflow_runs_claimable
ON kagzi.workflow_runs (task_queue, namespace_id, COALESCE(wake_up_at, created_at))
WHERE status IN ('PENDING', 'SLEEPING');

-- Index for step retries
CREATE INDEX IF NOT EXISTS idx_step_runs_pending_retry
ON kagzi.step_runs (retry_at)
WHERE status = 'PENDING' AND retry_at IS NOT NULL;
