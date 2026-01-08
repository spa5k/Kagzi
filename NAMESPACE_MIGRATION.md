# Namespace API Migration Summary

## Overview
The namespace API has been updated to use `namespace` (string identifier) instead of `namespaceId` as the primary field for referencing namespaces. Additionally, namespaces now support enable/disable functionality instead of deletion.

## API Changes

### Proto Changes (namespace.proto)

#### Namespace Message
- Added `id` field: Internal UUID primary key
- Renamed `namespaceId` → `namespace`: Unique identifier (e.g., "production", "staging")
- Added `enabled` field: Boolean flag to enable/disable namespace

#### Service Changes
- **Removed**: `DeleteNamespace` RPC (and `DeletionMode` enum)
- **Added**: `EnableNamespace` RPC
- **Added**: `DisableNamespace` RPC

### Field Renames
All requests/responses now use `namespace` instead of `namespaceId`:
- `CreateNamespaceRequest.namespace`
- `GetNamespaceRequest.namespace`
- `UpdateNamespaceRequest.namespace`
- `EnableNamespaceRequest.namespace`
- `DisableNamespaceRequest.namespace`
- And all workflow/worker/schedule requests

## Frontend Changes

### Updated Hooks (`use-grpc-services.ts`)
- Updated all namespace-related hooks to use `namespace` field
- Removed `useDeleteNamespace` hook
- Added `useEnableNamespace` hook
- Added `useDisableNamespace` hook
- Updated query keys to use `namespace` instead of `namespaceId`

### Updated Components
1. **namespace-switcher.tsx**
   - Now filters to show only enabled namespaces
   - Uses `namespace` field for identification

2. **use-namespace.tsx**
   - Removed `includeDeleted` parameter from ListNamespacesRequest

3. **app-sidebar.tsx**
   - Added "Namespaces" link under Admin section

### New Routes
- **`/namespaces`**: Full namespace management UI
  - List all namespaces (enabled and disabled)
  - Create new namespaces
  - Edit namespace display name and description
  - Enable/disable namespaces (not delete)
  - Visual indicators for enabled/disabled status

### Updated Route Files
All route files updated to use `namespace` instead of `namespaceId` in requests:
- `$namespaceId/workflows/$id.tsx`
- `$namespaceId/workflows/index.tsx`
- `$namespaceId/schedules/$id.tsx`

### API Queries (`lib/api-queries.ts`)
Updated parameter names from `namespaceId` to `namespace` in:
- `useListWorkflows`
- `useListSchedules`
- `useListWorkers`

## Behavioral Changes

### Namespace Management
- **Before**: Namespaces could be deleted with different deletion modes (fail if resources, soft delete, cascade)
- **After**: Namespaces can only be enabled or disabled
  - Disabled namespaces: Prevent new workflows from starting
  - Enabled namespaces: Allow normal operation
  - No deletion capability (safer for production)

### Namespace Switcher
- **Before**: Showed all namespaces including deleted ones (with flag)
- **After**: Shows only enabled namespaces by default
- Admins can view/manage all namespaces via the `/namespaces` route

## Migration Steps

If you have existing code using the old API:

1. **Replace field names**:
   ```typescript
   // Old
   { namespaceId: "production" }
   
   // New
   { namespace: "production" }
   ```

2. **Update namespace references**:
   ```typescript
   // Old
   namespace.namespaceId
   
   // New
   namespace.namespace
   ```

3. **Replace delete with disable**:
   ```typescript
   // Old
   await deleteNamespace({ namespaceId: "staging", mode: DeletionMode.SOFT_DELETE });
   
   // New
   await disableNamespace({ namespace: "staging" });
   ```

4. **Check for enabled status**:
   ```typescript
   // New field available
   if (namespace.enabled) {
     // namespace is active
   }
   ```

## Benefits

1. **Safer**: No accidental deletions of namespace data
2. **Reversible**: Can re-enable disabled namespaces
3. **Clearer**: `namespace` is more intuitive than `namespaceId`
4. **Better UX**: Visual indicators for enabled/disabled status
5. **Consistent**: Single string identifier throughout the API

## Testing

All TypeScript type checks pass:
```bash
cd app && npm run typecheck  # ✓ Success
```

All proto files regenerated successfully:
```bash
cd app && npm run generate  # ✓ Success
```
