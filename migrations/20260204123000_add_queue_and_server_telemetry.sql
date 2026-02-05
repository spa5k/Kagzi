-- Queue and server telemetry tables + server-derived worker fields.

ALTER TABLE kagzi.worker_telemetry_state
    ADD COLUMN IF NOT EXISTS active_workflows_authoritative INTEGER NOT NULL DEFAULT 0;

ALTER TABLE kagzi.worker_telemetry_state
    ADD COLUMN IF NOT EXISTS last_claim_at TIMESTAMPTZ;

ALTER TABLE kagzi.worker_telemetry_state
    ADD COLUMN IF NOT EXISTS last_claim_result TEXT NOT NULL DEFAULT '';

ALTER TABLE kagzi.worker_telemetry_state
    ADD COLUMN IF NOT EXISTS last_claim_error TEXT NOT NULL DEFAULT '';

CREATE TABLE IF NOT EXISTS kagzi.queue_telemetry_state (
    namespace TEXT NOT NULL,
    task_queue TEXT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    -- Depth gauges (server-derived)
    pending_count BIGINT NOT NULL DEFAULT 0,
    sleeping_count BIGINT NOT NULL DEFAULT 0,
    running_count BIGINT NOT NULL DEFAULT 0,
    due_count BIGINT NOT NULL DEFAULT 0,

    -- Publish effectiveness
    publish_attempts BIGINT NOT NULL DEFAULT 0,
    publish_errors BIGINT NOT NULL DEFAULT 0,
    last_publish_ok_at TIMESTAMPTZ,
    last_publish_error_at TIMESTAMPTZ,
    last_publish_error TEXT NOT NULL DEFAULT '',

    -- Coordinator notification
    last_due_work_notified_at TIMESTAMPTZ,

    -- Forward-compatible extras
    extra JSONB NOT NULL DEFAULT '{}'::jsonb,

    PRIMARY KEY (namespace, task_queue)
);

CREATE INDEX IF NOT EXISTS idx_queue_telemetry_state_namespace_updated
    ON kagzi.queue_telemetry_state (namespace, updated_at DESC);

CREATE TABLE IF NOT EXISTS kagzi.server_telemetry_events (
    event_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    namespace TEXT NOT NULL DEFAULT '',
    occurred_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    level TEXT NOT NULL DEFAULT 'info',
    event_type TEXT NOT NULL,
    message TEXT NOT NULL DEFAULT '',
    extra JSONB NOT NULL DEFAULT '{}'::jsonb
);

CREATE INDEX IF NOT EXISTS idx_server_telemetry_events_time
    ON kagzi.server_telemetry_events (occurred_at DESC);

