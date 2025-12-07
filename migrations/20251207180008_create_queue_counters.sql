-- Add migration script here
CREATE TABLE IF NOT EXISTS kagzi.queue_counters (
    namespace_id TEXT NOT NULL,
    task_queue TEXT NOT NULL,
    workflow_type TEXT NOT NULL,
    active_count INT NOT NULL DEFAULT 0,
    PRIMARY KEY (namespace_id, task_queue, workflow_type)
);

CREATE INDEX IF NOT EXISTS idx_queue_counters_lookup
    ON kagzi.queue_counters (namespace_id, task_queue);

