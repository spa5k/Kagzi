-- Migration 006: Add Production-Ready Worker Management
-- Enables worker lifecycle tracking, health monitoring, and graceful shutdown
-- Part of V2 Feature 5: Production-Ready Workers

-- Create workers table for tracking active workers
CREATE TABLE workers (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    worker_name TEXT NOT NULL UNIQUE,
    hostname TEXT,
    process_id INTEGER,
    started_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_heartbeat TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    status TEXT NOT NULL CHECK (status IN ('RUNNING', 'SHUTTING_DOWN', 'STOPPED')),
    config JSONB,
    metadata JSONB,
    stopped_at TIMESTAMPTZ
);

-- Index for finding active workers
CREATE INDEX idx_workers_status_heartbeat
ON workers(status, last_heartbeat);

-- Index for worker name lookup
CREATE INDEX idx_workers_name
ON workers(worker_name);

-- Create worker_events table for lifecycle event tracking
CREATE TABLE worker_events (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    worker_id UUID NOT NULL REFERENCES workers(id) ON DELETE CASCADE,
    event_type TEXT NOT NULL CHECK (event_type IN ('STARTED', 'HEARTBEAT', 'SHUTDOWN_REQUESTED', 'SHUTDOWN_COMPLETE', 'CRASHED', 'WORKFLOW_STARTED', 'WORKFLOW_COMPLETED', 'WORKFLOW_FAILED')),
    event_data JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Index for finding events by worker
CREATE INDEX idx_worker_events_worker_id
ON worker_events(worker_id, created_at DESC);

-- Index for finding events by type
CREATE INDEX idx_worker_events_type
ON worker_events(event_type, created_at DESC);

-- Add worker_name to worker_leases for better tracking
ALTER TABLE worker_leases
ADD COLUMN worker_name TEXT;

-- Index for lease cleanup by worker
CREATE INDEX idx_worker_leases_worker_name
ON worker_leases(worker_name);

-- Add comments for documentation
COMMENT ON TABLE workers IS 'Tracks active and historical worker instances';
COMMENT ON COLUMN workers.worker_name IS 'Unique identifier for the worker instance';
COMMENT ON COLUMN workers.last_heartbeat IS 'Last time this worker sent a heartbeat signal';
COMMENT ON COLUMN workers.status IS 'Current status of the worker (RUNNING, SHUTTING_DOWN, STOPPED)';
COMMENT ON COLUMN workers.config IS 'Worker configuration (poll interval, concurrency limits, etc.)';
COMMENT ON COLUMN workers.metadata IS 'Additional metadata (Rust version, Kagzi version, etc.)';

COMMENT ON TABLE worker_events IS 'Audit log of worker lifecycle events';
COMMENT ON COLUMN worker_events.event_type IS 'Type of event (STARTED, HEARTBEAT, SHUTDOWN_*, WORKFLOW_*)';
COMMENT ON COLUMN worker_events.event_data IS 'Additional event-specific data';

COMMENT ON COLUMN worker_leases.worker_name IS 'Name of the worker holding this lease';
