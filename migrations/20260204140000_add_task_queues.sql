-- Logical task queue registry (metadata only).
--
-- Queues are scoped to namespaces and map to underlying queue backends (Kafka/NATS/Postgres/etc.)
-- by {namespace, task_queue}. This table exists for UX/governance and does not provision broker resources.

CREATE TABLE IF NOT EXISTS kagzi.task_queues (
    namespace TEXT NOT NULL REFERENCES kagzi.namespaces(namespace) ON DELETE RESTRICT ON UPDATE CASCADE,
    task_queue TEXT NOT NULL,

    display_name TEXT,
    description TEXT,
    labels JSONB NOT NULL DEFAULT '{}'::jsonb,
    extra JSONB NOT NULL DEFAULT '{}'::jsonb,

    enabled BOOLEAN NOT NULL DEFAULT TRUE,

    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    PRIMARY KEY (namespace, task_queue)
);

CREATE INDEX IF NOT EXISTS idx_task_queues_namespace_enabled
    ON kagzi.task_queues (namespace, enabled);

-- Ensure every namespace has a default queue registered.
INSERT INTO kagzi.task_queues (namespace, task_queue, display_name, description, enabled)
SELECT n.namespace, 'default', 'Default', 'Default task queue', TRUE
FROM kagzi.namespaces n
WHERE n.deleted_at IS NULL
ON CONFLICT (namespace, task_queue) DO NOTHING;

