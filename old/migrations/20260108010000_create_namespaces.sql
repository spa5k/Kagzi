-- Create namespaces table to make namespace a first-class entity
CREATE TABLE kagzi.namespaces (
    namespace_id TEXT PRIMARY KEY,
    display_name TEXT NOT NULL,
    description TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    deleted_at TIMESTAMPTZ
);

-- Create index for soft-delete queries
CREATE INDEX idx_namespaces_active ON kagzi.namespaces (namespace_id) WHERE deleted_at IS NULL;

-- Insert default namespace to maintain referential integrity with existing data
INSERT INTO kagzi.namespaces (namespace_id, display_name, description)
VALUES ('default', 'Default', 'Default namespace for workflows and workers')
ON CONFLICT (namespace_id) DO NOTHING;

-- Add foreign key constraints to existing tables (optional, can be added later for stricter enforcement)
-- Note: We're not adding FK constraints immediately to avoid breaking existing workflows
-- This allows for a gradual migration approach
