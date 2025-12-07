-- Up
CREATE TABLE IF NOT EXISTS kagzi.workflow_payloads (
    run_id UUID PRIMARY KEY REFERENCES kagzi.workflow_runs(run_id) ON DELETE CASCADE,
    input JSONB NOT NULL,
    output JSONB,
    context JSONB,
    created_at TIMESTAMPTZ DEFAULT NOW()
);

