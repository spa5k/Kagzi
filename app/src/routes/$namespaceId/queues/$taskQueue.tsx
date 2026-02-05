import { Badge } from "@/components/ui/badge";
import { QueryError } from "@/components/ui/query-error";
import { useGetQueueTelemetryState } from "@/lib/api-queries";
import { decodeJsonBytes } from "@/lib/utils";
import { useGetQueue } from "@/hooks/use-grpc-services";
import { createFileRoute, Link, useParams } from "@tanstack/react-router";
import { type Timestamp, timestampDate } from "@bufbuild/protobuf/wkt";

function formatRelativeTime(timestamp: Timestamp | undefined): string {
  if (!timestamp) return "—";
  const date = timestampDate(timestamp);
  const now = new Date();
  const diffMs = now.getTime() - date.getTime();
  const diffSecs = Math.floor(diffMs / 1000);
  const diffMins = Math.floor(diffSecs / 60);
  const diffHours = Math.floor(diffMins / 60);

  if (diffHours > 0) return `${diffHours}h ago`;
  if (diffMins > 0) return `${diffMins}m ago`;
  if (diffSecs > 0) return `${diffSecs}s ago`;
  return "just now";
}

export const Route = createFileRoute("/$namespaceId/queues/$taskQueue")({
  component: QueueDetailPage,
});

function QueueDetailPage() {
  const params = useParams({ from: "/$namespaceId/queues/$taskQueue" });
  const namespace = params.namespaceId;
  const taskQueue = params.taskQueue;

  const queueQuery = useGetQueue({ namespace, taskQueue });
  const telemetryQuery = useGetQueueTelemetryState(namespace, taskQueue);

  const error = queueQuery.error || telemetryQuery.error;
  const isLoading = queueQuery.isLoading || telemetryQuery.isLoading;
  const refetch = () => {
    void queueQuery.refetch();
    void telemetryQuery.refetch();
  };

  if (error) {
    return (
      <div className="h-full flex items-center justify-center p-6 bg-background">
        <QueryError error={error} onRetry={refetch} className="max-w-md" />
      </div>
    );
  }

  if (isLoading) {
    return (
      <div className="p-6 space-y-4">
        <div className="h-7 w-80 rounded bg-muted animate-pulse" />
        <div className="h-24 rounded bg-muted/30 animate-pulse" />
        <div className="h-32 rounded bg-muted/20 animate-pulse" />
      </div>
    );
  }

  const meta = queueQuery.data?.queue;
  const state = telemetryQuery.data?.state;
  if (!meta && !state) {
    return (
      <div className="p-6">
        <p className="text-sm text-muted-foreground">Queue not found.</p>
      </div>
    );
  }

  const extraTelemetry = decodeJsonBytes<Record<string, unknown>>(state?.extraJson);
  const extraMeta = decodeJsonBytes<Record<string, unknown>>(meta?.extraJson);
  const extra = extraTelemetry || extraMeta;
  const hasError = !!state?.lastPublishError;
  const enabled = meta?.enabled ?? true;

  return (
    <div className="p-6 space-y-6">
      <div className="flex items-start justify-between">
        <div>
          <div className="flex items-center gap-2">
            <h1 className="text-xl font-medium font-mono">
              {meta?.displayName || state?.taskQueue || taskQueue}
            </h1>
            {!enabled && <Badge variant="secondary">disabled</Badge>}
            {hasError && <Badge variant="destructive">publish errors</Badge>}
          </div>
          <div className="mt-2">
            <Link
              to="/$namespaceId/queues"
              params={{ namespaceId: namespace }}
              className="text-xs text-muted-foreground hover:underline"
            >
              ← Back to queues
            </Link>
          </div>
        </div>
        <div className="text-right text-xs text-muted-foreground space-y-1">
          <div>Updated: {formatRelativeTime(state?.updatedAt || meta?.updatedAt)}</div>
          <div>Due notified: {formatRelativeTime(state?.lastDueWorkNotifiedAt)}</div>
          <div>Last publish ok: {formatRelativeTime(state?.lastPublishOkAt)}</div>
          <div>Last publish error: {formatRelativeTime(state?.lastPublishErrorAt)}</div>
        </div>
      </div>

      {meta?.description && (
        <div className="text-sm text-muted-foreground whitespace-pre-wrap">{meta.description}</div>
      )}

      <div className="border border-border p-4 rounded-md">
        <div className="grid grid-cols-6 gap-4 text-sm">
          <div>
            <dt className="text-xs text-muted-foreground">Pending</dt>
            <dd className="font-mono">{state?.pendingCount ?? 0}</dd>
          </div>
          <div>
            <dt className="text-xs text-muted-foreground">Sleeping</dt>
            <dd className="font-mono">{state?.sleepingCount ?? 0}</dd>
          </div>
          <div>
            <dt className="text-xs text-muted-foreground">Running</dt>
            <dd className="font-mono">{state?.runningCount ?? 0}</dd>
          </div>
          <div>
            <dt className="text-xs text-muted-foreground">Due</dt>
            <dd className="font-mono">{state?.dueCount ?? 0}</dd>
          </div>
          <div>
            <dt className="text-xs text-muted-foreground">Publish attempts</dt>
            <dd className="font-mono">{state?.publishAttempts ?? 0}</dd>
          </div>
          <div>
            <dt className="text-xs text-muted-foreground">Publish errors</dt>
            <dd className="font-mono">{state?.publishErrors ?? 0}</dd>
          </div>
        </div>

        {state?.lastPublishError && (
          <div className="mt-4 text-xs font-mono text-destructive truncate">
            last_publish_error: {state.lastPublishError}
          </div>
        )}
      </div>

      {extra && Object.keys(extra).length > 0 && (
        <div>
          <div className="text-sm font-medium mb-2">Extra</div>
          <pre className="text-xs font-mono bg-muted/20 border border-border rounded-md p-3 overflow-auto">
            {JSON.stringify(extra, null, 2)}
          </pre>
        </div>
      )}
    </div>
  );
}
