import { Badge } from "@/components/ui/badge";
import { QueryError } from "@/components/ui/query-error";
import { useGetServerInfo, useGetStats, useHealthCheck } from "@/hooks/use-grpc-services";
import { useListQueueTelemetryStates, useListServerTelemetryEvents } from "@/lib/api-queries";
import { ServingStatus, ServingStatusLabel, TelemetryLevel, TelemetryLevelLabel } from "@/types";
import { createFileRoute, Link, useParams } from "@tanstack/react-router";
import { type Timestamp, timestampDate } from "@bufbuild/protobuf/wkt";

function formatInt(value: bigint | number | undefined): string {
  if (value === undefined) return "—";
  return value.toString();
}

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

function getLevelVariant(level: number): "default" | "secondary" | "destructive" | "outline" {
  switch (level) {
    case TelemetryLevel.ERROR:
      return "destructive";
    case TelemetryLevel.WARN:
      return "secondary";
    case TelemetryLevel.INFO:
      return "default";
    default:
      return "outline";
  }
}

export const Route = createFileRoute("/$namespaceId/server")({
  component: ServerPage,
});

function ServerPage() {
  const params = useParams({ from: "/$namespaceId/server" });
  const namespace = params.namespaceId;

  const healthQuery = useHealthCheck();
  const serverInfoQuery = useGetServerInfo();
  const statsQuery = useGetStats({ namespace });

  const queueStatesQuery = useListQueueTelemetryStates(namespace);
  const eventsQuery = useListServerTelemetryEvents(namespace);

  const error =
    healthQuery.error ||
    serverInfoQuery.error ||
    statsQuery.error ||
    queueStatesQuery.error ||
    eventsQuery.error;

  const isLoading =
    healthQuery.isLoading ||
    serverInfoQuery.isLoading ||
    statsQuery.isLoading ||
    queueStatesQuery.isLoading ||
    eventsQuery.isLoading;

  const refetch = () => {
    void healthQuery.refetch();
    void serverInfoQuery.refetch();
    void statsQuery.refetch();
    void queueStatesQuery.refetch();
    void eventsQuery.refetch();
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
        <div className="h-8 w-48 rounded bg-muted animate-pulse" />
        <div className="h-5 w-80 rounded bg-muted/50 animate-pulse" />
        <div className="h-28 rounded bg-muted/20 animate-pulse" />
        <div className="h-56 rounded bg-muted/20 animate-pulse" />
      </div>
    );
  }

  const health = healthQuery.data;
  const info = serverInfoQuery.data;
  const stats = statsQuery.data;
  const queueStates = queueStatesQuery.data?.states ?? [];
  const events = eventsQuery.data?.events ?? [];

  const dueTotal = queueStates.reduce((acc, s) => acc + Number(s.dueCount), 0);
  const pendingTotal = queueStates.reduce((acc, s) => acc + Number(s.pendingCount), 0);
  const publishErrorsTotal = queueStates.reduce((acc, s) => acc + Number(s.publishErrors), 0);

  return (
    <div className="p-6 space-y-6">
      <div className="flex items-start justify-between gap-4">
        <div>
          <h1 className="text-xl font-medium">Server</h1>
          <p className="text-sm text-muted-foreground">Namespace: {namespace}</p>
        </div>

        <div className="flex items-center gap-2">
          <Badge variant={health?.status === ServingStatus.SERVING ? "default" : "destructive"}>
            {ServingStatusLabel[health?.status ?? ServingStatus.UNSPECIFIED]}
          </Badge>
          <Link
            to="/$namespaceId/events"
            params={{ namespaceId: namespace }}
            className="text-xs font-mono text-muted-foreground hover:text-foreground"
          >
            view events →
          </Link>
        </div>
      </div>

      <div className="border border-border rounded-md p-4 space-y-4">
        <div className="grid grid-cols-2 md:grid-cols-4 gap-4 text-sm">
          <div>
            <dt className="text-xs text-muted-foreground">Version</dt>
            <dd className="font-mono">{info?.version || "—"}</dd>
          </div>
          <div>
            <dt className="text-xs text-muted-foreground">API Version</dt>
            <dd className="font-mono">{info?.apiVersion || "—"}</dd>
          </div>
          <div>
            <dt className="text-xs text-muted-foreground">Min SDK</dt>
            <dd className="font-mono">{info?.minSdkVersion || "—"}</dd>
          </div>
          <div>
            <dt className="text-xs text-muted-foreground">Health message</dt>
            <dd className="font-mono truncate">{health?.message || "—"}</dd>
          </div>
        </div>

        {info?.supportedFeatures && info.supportedFeatures.length > 0 && (
          <div>
            <div className="text-xs text-muted-foreground mb-2">Supported features</div>
            <div className="flex flex-wrap gap-1">
              {info.supportedFeatures.map((f) => (
                <span key={f} className="px-2 py-0.5 bg-muted text-xs font-mono">
                  {f}
                </span>
              ))}
            </div>
          </div>
        )}
      </div>

      <div className="border border-border rounded-md p-4">
        <div className="text-sm font-medium mb-3">Namespace summary</div>
        <div className="grid grid-cols-2 md:grid-cols-5 gap-4 text-sm">
          <div>
            <dt className="text-xs text-muted-foreground">Workflows</dt>
            <dd className="font-mono">{formatInt(stats?.totalWorkflows)}</dd>
          </div>
          <div>
            <dt className="text-xs text-muted-foreground">Running</dt>
            <dd className="font-mono">{formatInt(stats?.runningWorkflows)}</dd>
          </div>
          <div>
            <dt className="text-xs text-muted-foreground">Pending</dt>
            <dd className="font-mono">{formatInt(stats?.pendingWorkflows)}</dd>
          </div>
          <div>
            <dt className="text-xs text-muted-foreground">Workers (online/total)</dt>
            <dd className="font-mono">
              {formatInt(stats?.onlineWorkers)}/{formatInt(stats?.totalWorkers)}
            </dd>
          </div>
          <div>
            <dt className="text-xs text-muted-foreground">Schedules (enabled/total)</dt>
            <dd className="font-mono">
              {formatInt(stats?.enabledSchedules)}/{formatInt(stats?.totalSchedules)}
            </dd>
          </div>
        </div>

        <div className="mt-4 grid grid-cols-2 md:grid-cols-5 gap-4 text-sm">
          <div>
            <dt className="text-xs text-muted-foreground">Queues tracked</dt>
            <dd className="font-mono">{queueStates.length}</dd>
          </div>
          <div>
            <dt className="text-xs text-muted-foreground">Due</dt>
            <dd className="font-mono">{dueTotal}</dd>
          </div>
          <div>
            <dt className="text-xs text-muted-foreground">Pending</dt>
            <dd className="font-mono">{pendingTotal}</dd>
          </div>
          <div>
            <dt className="text-xs text-muted-foreground">Publish errors</dt>
            <dd className="font-mono">{publishErrorsTotal}</dd>
          </div>
          <div>
            <dt className="text-xs text-muted-foreground">Events (errors)</dt>
            <dd className="font-mono">
              {events.length} ({events.filter((e) => e.level === TelemetryLevel.ERROR).length})
            </dd>
          </div>
        </div>
      </div>

      <div>
        <div className="flex items-end justify-between mb-2">
          <div className="text-sm font-medium">Recent server events</div>
          <Link
            to="/$namespaceId/events"
            params={{ namespaceId: namespace }}
            className="font-mono text-xs uppercase tracking-wider text-muted-foreground hover:text-primary transition-colors"
          >
            View all →
          </Link>
        </div>

        {events.length === 0 ? (
          <p className="text-xs text-muted-foreground">No events recorded.</p>
        ) : (
          <div className="border border-border rounded-md divide-y divide-border">
            {events.slice(0, 50).map((e, idx) => (
              <div key={`${e.occurredAt?.seconds ?? 0}-${idx}`} className="p-3 flex gap-3">
                <div className="w-28 text-xs text-muted-foreground font-mono shrink-0">
                  {e.occurredAt ? formatRelativeTime(e.occurredAt) : "—"}
                </div>
                <div className="shrink-0">
                  <Badge variant={getLevelVariant(e.level)}>{TelemetryLevelLabel[e.level]}</Badge>
                </div>
                <div className="min-w-0 flex-1">
                  <div className="text-xs font-mono">{e.eventType}</div>
                  {e.message && (
                    <div className="text-xs text-muted-foreground truncate font-mono">
                      {e.message}
                    </div>
                  )}
                </div>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
