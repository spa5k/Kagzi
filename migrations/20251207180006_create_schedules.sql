CREATE TABLE IF NOT EXISTS kagzi.schedules (
    schedule_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    namespace_id TEXT NOT NULL DEFAULT 'default',
    task_queue TEXT NOT NULL,
    workflow_type TEXT NOT NULL,
    cron_expr TEXT NOT NULL,
    input JSONB NOT NULL DEFAULT '{}',
    context JSONB,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    max_catchup INT NOT NULL DEFAULT 100,
    next_fire_at TIMESTAMPTZ NOT NULL,
    last_fired_at TIMESTAMPTZ,
    version TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_schedules_due
    ON kagzi.schedules (namespace_id, next_fire_at)
    WHERE enabled = TRUE;

CREATE INDEX IF NOT EXISTS idx_schedules_ns_queue
    ON kagzi.schedules (namespace_id, task_queue);

CREATE TABLE IF NOT EXISTS kagzi.schedule_firings (
    id BIGSERIAL PRIMARY KEY,
    schedule_id UUID NOT NULL REFERENCES kagzi.schedules(schedule_id) ON DELETE CASCADE,
    fire_at TIMESTAMPTZ NOT NULL,
    run_id UUID NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (schedule_id, fire_at)
);

