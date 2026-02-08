CREATE TABLE IF NOT EXISTS kagzi.workers (
    worker_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    namespace_id TEXT NOT NULL DEFAULT 'default',
    task_queue TEXT NOT NULL,
    hostname TEXT,
    pid INTEGER,
    version TEXT,
    workflow_types TEXT[] NOT NULL DEFAULT '{}',
    max_concurrent INTEGER NOT NULL DEFAULT 100,
    status TEXT NOT NULL DEFAULT 'ONLINE',
    active_count INTEGER NOT NULL DEFAULT 0,
    total_completed BIGINT NOT NULL DEFAULT 0,
    total_failed BIGINT NOT NULL DEFAULT 0,
    registered_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_heartbeat_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    deregistered_at TIMESTAMPTZ,
    labels JSONB DEFAULT '{}'
);

CREATE INDEX IF NOT EXISTS idx_workers_queue
    ON kagzi.workers (namespace_id, task_queue, status)
    WHERE status = 'ONLINE';

CREATE INDEX IF NOT EXISTS idx_workers_heartbeat
    ON kagzi.workers (status, last_heartbeat_at)
    WHERE status != 'OFFLINE';

CREATE UNIQUE INDEX IF NOT EXISTS idx_workers_active_unique
    ON kagzi.workers (namespace_id, task_queue, hostname, pid)
    WHERE status != 'OFFLINE';

