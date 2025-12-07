-- Up
CREATE TABLE IF NOT EXISTS kagzi.step_runs (
    attempt_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    run_id UUID REFERENCES kagzi.workflow_runs(run_id),
    step_id TEXT NOT NULL,
    namespace_id TEXT NOT NULL DEFAULT 'default',
    status TEXT NOT NULL,
    output JSONB,
    error TEXT,
    attempts INTEGER NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    started_at TIMESTAMPTZ,
    finished_at TIMESTAMPTZ,
    retry_at TIMESTAMPTZ,
    retry_policy JSONB,
    attempt_number INTEGER NOT NULL DEFAULT 1,
    is_latest BOOLEAN DEFAULT true,
    input JSONB,
    child_workflow_run_id UUID,
    step_kind TEXT NOT NULL DEFAULT 'FUNCTION'
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_step_runs_latest
    ON kagzi.step_runs (run_id, step_id)
    WHERE is_latest = true;

CREATE INDEX IF NOT EXISTS idx_step_runs_history
    ON kagzi.step_runs (run_id, step_id, attempt_number);

CREATE INDEX IF NOT EXISTS idx_step_runs_retry
    ON kagzi.step_runs (run_id, retry_at)
    WHERE retry_at IS NOT NULL;

CREATE OR REPLACE FUNCTION kagzi.create_step_attempt(
    p_run_id UUID,
    p_step_id TEXT,
    p_attempt_number INTEGER DEFAULT 1,
    p_input JSONB DEFAULT NULL,
    p_status TEXT DEFAULT 'PENDING'
) RETURNS UUID AS $$
DECLARE
    v_attempt_id UUID;
BEGIN
    UPDATE kagzi.step_runs
    SET is_latest = false
    WHERE run_id = p_run_id AND step_id = p_step_id;

    INSERT INTO kagzi.step_runs (
        run_id, step_id, attempt_number, is_latest, input, created_at, status
    ) VALUES (
        p_run_id, p_step_id, p_attempt_number, true, p_input, NOW(), p_status
    ) RETURNING attempt_id INTO v_attempt_id;

    RETURN v_attempt_id;
END;
$$ LANGUAGE plpgsql;
