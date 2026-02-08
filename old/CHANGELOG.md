# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Breaking Changes

- **Removed `deadline_at` field** - This field was documented but never enforced. It has been removed from the API, database schema, and all client libraries.
- **Removed `idempotency_suffix` column** - Scheduled workflows now use compound `external_id` format instead of a separate suffix column.

### Changed

- **Scheduler idempotency**: Scheduled workflows now generate unique `external_id` values in the format `{schedule_id}:{fire_time_rfc3339}` (e.g., `schedule-abc:2025-12-28T00:00:00Z`). This eliminates the need for a separate `idempotency_suffix` column.
- **Constants standardization**: Introduced `DEFAULT_NAMESPACE` and `DEFAULT_VERSION` constants to replace hardcoded strings throughout the codebase.

### Migration Guide

#### For Scheduled Workflows

- **No action required** - The new external_id format applies automatically to new schedule fires
- Existing workflows in the database are unaffected
- The unique constraint now uses only `(namespace_id, external_id)` instead of `(namespace_id, external_id, idempotency_suffix)`

#### For deadline_at Users

- If you were passing `deadline_at` in `StartWorkflowRequest`, simply remove this field
- The field was never enforced, so removing it has no behavioral impact
- If you need deadline enforcement, this feature may be added in a future release

#### Database Migration

Run the following migrations in order:

1. `20251228171027_remove_idempotency_suffix.sql` - Removes idempotency_suffix column and updates unique index
2. `20251228171103_remove_deadline_at.sql` - Removes deadline_at column

Migrations will run automatically on server startup.
