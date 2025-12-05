-- Worker registry table
CREATE TABLE kagzi.workers (
    worker_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    namespace_id TEXT NOT NULL DEFAULT 'default',
    task_queue TEXT NOT NULL,

    -- Identity
    hostname TEXT,
    pid INTEGER,
    version TEXT,

    -- Capabilities
    workflow_types TEXT[] NOT NULL DEFAULT '{}',
    max_concurrent INTEGER NOT NULL DEFAULT 100,

    -- State
    status TEXT NOT NULL DEFAULT 'ONLINE',  -- ONLINE, DRAINING, OFFLINE
    active_count INTEGER NOT NULL DEFAULT 0,

    -- Metrics (cumulative)
    total_completed BIGINT NOT NULL DEFAULT 0,
    total_failed BIGINT NOT NULL DEFAULT 0,

    -- Timestamps
    registered_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_heartbeat_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    deregistered_at TIMESTAMPTZ,

    -- Extensibility
    labels JSONB DEFAULT '{}'
);

-- Fast lookup by queue
CREATE INDEX idx_workers_queue
ON kagzi.workers (namespace_id, task_queue, status)
WHERE status = 'ONLINE';

-- Fast stale detection
CREATE INDEX idx_workers_heartbeat
ON kagzi.workers (status, last_heartbeat_at)
WHERE status != 'OFFLINE';

-- Enforce uniqueness for active workers
CREATE UNIQUE INDEX idx_workers_active_unique
ON kagzi.workers (namespace_id, task_queue, hostname, pid)
WHERE status != 'OFFLINE';

