-- Queue-level and workflow-type-level concurrency limits supplied by workers.
CREATE TABLE IF NOT EXISTS kagzi.queue_configs (
    namespace_id TEXT NOT NULL,
    task_queue TEXT NOT NULL,
    max_concurrent INT,
    updated_at TIMESTAMPTZ DEFAULT NOW(),
    PRIMARY KEY (namespace_id, task_queue)
);

CREATE TABLE IF NOT EXISTS kagzi.workflow_type_configs (
    namespace_id TEXT NOT NULL,
    task_queue TEXT NOT NULL,
    workflow_type TEXT NOT NULL,
    max_concurrent INT,
    updated_at TIMESTAMPTZ DEFAULT NOW(),
    PRIMARY KEY (namespace_id, task_queue, workflow_type)
);
