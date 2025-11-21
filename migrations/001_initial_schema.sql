-- Initial Kagzi Schema
-- This creates the core tables needed for durable workflow execution

-- Workflow Runs: Represents a single execution instance of a workflow
CREATE TABLE workflow_runs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    workflow_name TEXT NOT NULL,
    workflow_version TEXT DEFAULT 'v1' NOT NULL,
    input JSONB NOT NULL,
    output JSONB,
    status TEXT NOT NULL CHECK (status IN ('PENDING', 'RUNNING', 'COMPLETED', 'FAILED', 'SLEEPING', 'CANCELLED')),
    error TEXT,
    created_at TIMESTAMPTZ DEFAULT NOW() NOT NULL,
    updated_at TIMESTAMPTZ DEFAULT NOW() NOT NULL,
    sleep_until TIMESTAMPTZ,
    completed_at TIMESTAMPTZ
);

-- Indexes for efficient querying
CREATE INDEX idx_workflow_runs_status ON workflow_runs(status);
CREATE INDEX idx_workflow_runs_sleep_until ON workflow_runs(sleep_until) WHERE status = 'SLEEPING';
CREATE INDEX idx_workflow_runs_created_at ON workflow_runs(created_at);
CREATE INDEX idx_workflow_runs_workflow_name ON workflow_runs(workflow_name);

-- Step Runs: Memoized results of individual steps within a workflow
CREATE TABLE step_runs (
    workflow_run_id UUID NOT NULL REFERENCES workflow_runs(id) ON DELETE CASCADE,
    step_id TEXT NOT NULL,
    input_hash TEXT,
    output JSONB,
    error TEXT,
    status TEXT NOT NULL CHECK (status IN ('COMPLETED', 'FAILED')),
    attempts INT DEFAULT 1 NOT NULL,
    created_at TIMESTAMPTZ DEFAULT NOW() NOT NULL,
    completed_at TIMESTAMPTZ,
    PRIMARY KEY (workflow_run_id, step_id)
);

CREATE INDEX idx_step_runs_workflow_run_id ON step_runs(workflow_run_id);
CREATE INDEX idx_step_runs_status ON step_runs(status);

-- Worker Leases: Ensures only one worker processes a workflow at a time
CREATE TABLE worker_leases (
    workflow_run_id UUID PRIMARY KEY REFERENCES workflow_runs(id) ON DELETE CASCADE,
    worker_id TEXT NOT NULL,
    acquired_at TIMESTAMPTZ DEFAULT NOW() NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    heartbeat_at TIMESTAMPTZ DEFAULT NOW() NOT NULL
);

CREATE INDEX idx_worker_leases_expires_at ON worker_leases(expires_at);
CREATE INDEX idx_worker_leases_worker_id ON worker_leases(worker_id);

-- Function to automatically update updated_at timestamp
CREATE OR REPLACE FUNCTION update_updated_at_column()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ language 'plpgsql';

-- Trigger to auto-update updated_at on workflow_runs
CREATE TRIGGER update_workflow_runs_updated_at
    BEFORE UPDATE ON workflow_runs
    FOR EACH ROW
    EXECUTE FUNCTION update_updated_at_column();
