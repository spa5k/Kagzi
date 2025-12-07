-- Up
CREATE TABLE IF NOT EXISTS kagzi.workflow_runs (
    run_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    namespace_id TEXT NOT NULL DEFAULT 'default',
    external_id TEXT NOT NULL,
    task_queue TEXT NOT NULL,
    workflow_type TEXT NOT NULL,
    status TEXT NOT NULL,
    locked_by TEXT,
    locked_until TIMESTAMPTZ,
    attempts INTEGER NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    started_at TIMESTAMPTZ,
    finished_at TIMESTAMPTZ,
    wake_up_at TIMESTAMPTZ,
    deadline_at TIMESTAMPTZ,
    retry_policy JSONB,
    parent_step_attempt_id TEXT,
    version TEXT,
    idempotency_suffix TEXT,
    error TEXT
);

CREATE UNIQUE INDEX IF NOT EXISTS uq_active_workflow
    ON kagzi.workflow_runs (namespace_id, external_id, COALESCE(idempotency_suffix, ''))
    WHERE status NOT IN ('COMPLETED', 'FAILED', 'CANCELLED');

CREATE INDEX IF NOT EXISTS idx_queue_poll
    ON kagzi.workflow_runs (namespace_id, task_queue, status, wake_up_at);

CREATE INDEX IF NOT EXISTS idx_queue_poll_hot
    ON kagzi.workflow_runs (namespace_id, task_queue, COALESCE(wake_up_at, created_at))
    WHERE status IN ('PENDING', 'SLEEPING');

CREATE INDEX IF NOT EXISTS idx_workflow_runs_running
    ON kagzi.workflow_runs (task_queue, namespace_id, workflow_type)
    WHERE status = 'RUNNING';