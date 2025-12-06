-- Partial index for fast COUNT(*) of RUNNING workflows during concurrency checks.
-- Note: For large production tables, consider running this with CONCURRENTLY outside migrations.
CREATE INDEX IF NOT EXISTS idx_workflow_runs_running
ON kagzi.workflow_runs (task_queue, namespace_id, workflow_type)
WHERE status = 'RUNNING';

