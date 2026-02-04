-- Helps the DB claim query when workers restrict by workflow_type (common when sharing a task_queue).
-- Complements idx_workflow_available (namespace, task_queue, available_at).
CREATE INDEX IF NOT EXISTS idx_workflow_available_by_type
ON kagzi.workflow_runs (namespace, task_queue, workflow_type, available_at)
WHERE (status = ANY (ARRAY['PENDING'::text, 'SLEEPING'::text, 'RUNNING'::text]));

