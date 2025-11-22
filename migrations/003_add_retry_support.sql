-- Migration 003: Add Retry Support
-- Adds retry capabilities to step execution with automatic retry policies
-- Part of V2 Feature 1: Automatic Retry Policies

-- Add id column to step_runs for foreign key references
ALTER TABLE step_runs
ADD COLUMN id BIGSERIAL;

-- Add retry tracking columns to step_runs
ALTER TABLE step_runs
ADD COLUMN next_retry_at TIMESTAMPTZ,
ADD COLUMN retry_policy JSONB;

-- Create step_attempts table to track retry history
CREATE TABLE step_attempts (
    id BIGSERIAL PRIMARY KEY,
    step_run_id BIGINT NOT NULL REFERENCES step_runs(id) ON DELETE CASCADE,
    attempt_number INTEGER NOT NULL,
    started_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at TIMESTAMPTZ,
    error JSONB,
    status TEXT NOT NULL CHECK (status IN ('running', 'succeeded', 'failed')),
    UNIQUE(step_run_id, attempt_number)
);

-- Add indexes for efficient retry queries
CREATE INDEX idx_step_runs_next_retry_at ON step_runs(next_retry_at) WHERE next_retry_at IS NOT NULL;
CREATE INDEX idx_step_runs_attempts ON step_runs(attempts);
CREATE INDEX idx_step_attempts_step_run_id ON step_attempts(step_run_id);
CREATE INDEX idx_step_attempts_status ON step_attempts(status);

-- Add comment documentation
COMMENT ON COLUMN step_runs.attempts IS 'Number of attempts made for this step (0 = not yet attempted)';
COMMENT ON COLUMN step_runs.next_retry_at IS 'Timestamp when the next retry should be attempted (NULL = no retry scheduled)';
COMMENT ON COLUMN step_runs.retry_policy IS 'JSONB containing retry policy configuration (type, max_attempts, backoff settings, etc.)';
COMMENT ON TABLE step_attempts IS 'Historical record of all retry attempts for step executions';
