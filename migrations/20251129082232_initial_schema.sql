-- Add migration script here

-- Schema creation handled by init.sql

-- Workflow Runs Table
CREATE TABLE kagzi.workflow_runs (
    run_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    namespace_id TEXT NOT NULL DEFAULT 'default',
    business_id TEXT NOT NULL,      -- User provided ID
    task_queue TEXT NOT NULL,       -- "default", "critical", etc.
    workflow_type TEXT NOT NULL,    -- "WelcomeEmail"
    status TEXT NOT NULL,           -- PENDING, RUNNING, SLEEPING, COMPLETED, FAILED
    
    -- Data
    input JSONB NOT NULL,
    output JSONB,
    context JSONB,                  -- Tracing/Metadata
    
    -- Concurrency & Reliability
    idempotency_key TEXT,
    locked_by TEXT,                 -- Worker ID
    locked_until TIMESTAMPTZ,
    attempts INTEGER DEFAULT 0 NOT NULL,
    
    -- Timings
    created_at TIMESTAMPTZ DEFAULT NOW(),
    started_at TIMESTAMPTZ,
    finished_at TIMESTAMPTZ,
    wake_up_at TIMESTAMPTZ,         -- For sleeping workflows
    deadline_at TIMESTAMPTZ
);

-- Unique constraint for idempotency per namespace
CREATE UNIQUE INDEX idx_workflow_idempotency 
ON kagzi.workflow_runs (namespace_id, idempotency_key) 
WHERE idempotency_key IS NOT NULL;

-- Polling index (critical for performance)
CREATE INDEX idx_queue_poll 
ON kagzi.workflow_runs (namespace_id, task_queue, status, wake_up_at);

-- Step Runs Table
CREATE TABLE kagzi.step_runs (
    run_id UUID REFERENCES kagzi.workflow_runs(run_id),
    step_id TEXT NOT NULL,
    namespace_id TEXT NOT NULL DEFAULT 'default',
    status TEXT NOT NULL,           -- COMPLETED, FAILED
    output JSONB,
    error TEXT,
    
    -- Retry tracking
    attempts INTEGER DEFAULT 0 NOT NULL,
    
    -- Timings
    created_at TIMESTAMPTZ DEFAULT NOW(),
    started_at TIMESTAMPTZ,
    finished_at TIMESTAMPTZ,
    
    PRIMARY KEY (run_id, step_id)
);