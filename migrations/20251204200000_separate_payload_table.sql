CREATE TABLE kagzi.workflow_payloads (
    run_id UUID PRIMARY KEY REFERENCES kagzi.workflow_runs(run_id) ON DELETE CASCADE,
    input JSONB NOT NULL,
    output JSONB,
    context JSONB,
    created_at TIMESTAMPTZ DEFAULT NOW()
);

INSERT INTO kagzi.workflow_payloads (run_id, input, output, context)
SELECT run_id, input, output, context
FROM kagzi.workflow_runs
WHERE input IS NOT NULL;

ALTER TABLE kagzi.workflow_runs
    DROP COLUMN input,
    DROP COLUMN output,
    DROP COLUMN context;

CREATE INDEX idx_queue_poll_hot ON kagzi.workflow_runs (namespace_id, task_queue, COALESCE(wake_up_at, created_at))
WHERE status IN ('PENDING', 'SLEEPING');
