-- Worker telemetry tables: latest state + bounded event history for UI/analytics.

CREATE TABLE IF NOT EXISTS kagzi.worker_telemetry_state (
    worker_id UUID PRIMARY KEY,
    namespace TEXT NOT NULL,
    task_queue TEXT NOT NULL,
    signal_backend TEXT NOT NULL DEFAULT '',
    subscribed BOOLEAN NOT NULL DEFAULT FALSE,
    subscription_state TEXT NOT NULL DEFAULT '',
    last_subscribe_ok_at TIMESTAMPTZ,
    last_wakeup_at TIMESTAMPTZ,
    max_concurrent INTEGER NOT NULL DEFAULT 0,
    in_flight INTEGER NOT NULL DEFAULT 0,
    last_error TEXT NOT NULL DEFAULT '',
    last_error_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    extra JSONB NOT NULL DEFAULT '{}'::jsonb
);

CREATE INDEX IF NOT EXISTS idx_worker_telemetry_state_namespace_updated
    ON kagzi.worker_telemetry_state (namespace, updated_at DESC);

CREATE INDEX IF NOT EXISTS idx_worker_telemetry_state_namespace_queue_updated
    ON kagzi.worker_telemetry_state (namespace, task_queue, updated_at DESC);

CREATE TABLE IF NOT EXISTS kagzi.worker_telemetry_events (
    event_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    worker_id UUID NOT NULL,
    namespace TEXT NOT NULL,
    task_queue TEXT NOT NULL,
    occurred_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    level TEXT NOT NULL DEFAULT 'info',
    event_type TEXT NOT NULL,
    message TEXT NOT NULL DEFAULT '',
    extra JSONB NOT NULL DEFAULT '{}'::jsonb
);

CREATE INDEX IF NOT EXISTS idx_worker_telemetry_events_worker_time
    ON kagzi.worker_telemetry_events (namespace, worker_id, occurred_at DESC);

CREATE INDEX IF NOT EXISTS idx_worker_telemetry_events_namespace_time
    ON kagzi.worker_telemetry_events (namespace, occurred_at DESC);

