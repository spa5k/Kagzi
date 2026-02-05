import { Badge } from "@/components/ui/badge";
import { QueryError } from "@/components/ui/query-error";
import { useListWorkerTelemetryStates, useListWorkers } from "@/lib/api-queries";
import { decodeJsonBytes } from "@/lib/utils";
import { WorkerStatus, WorkerStatusLabel } from "@/types";
import { createFileRoute, Link, useParams } from "@tanstack/react-router";
import { type Timestamp, timestampDate } from "@bufbuild/protobuf/wkt";
import * as React from "react";

function getStatusVariant(status: number): "default" | "secondary" | "destructive" | "outline" {
  switch (status) {
    case WorkerStatus.ONLINE:
      return "default";
    case WorkerStatus.DRAINING:
      return "secondary";
    case WorkerStatus.OFFLINE:
      return "destructive";
    default:
      return "outline";
  }
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

type WorkerExtra = {
  hostname?: string | null;
  pid?: number;
  version?: string | null;
  labels?: Record<string, string>;
};

export const Route = createFileRoute("/$namespaceId/workers/")({
  component: WorkersPage,
});

function WorkersPage() {
  const params = useParams({ from: "/$namespaceId/workers/" });
  const namespace = params.namespaceId;

  const workersQuery = useListWorkers(namespace);
  const telemetryQuery = useListWorkerTelemetryStates(namespace);

  const error = workersQuery.error || telemetryQuery.error;
  const isLoading = workersQuery.isLoading || telemetryQuery.isLoading;
  const refetch = () => {
    void workersQuery.refetch();
    void telemetryQuery.refetch();
  };

  const workers = workersQuery.data?.workersList ?? [];
  const snapshots = telemetryQuery.data?.snapshots ?? [];

  const telemetryByWorkerId = React.useMemo(() => {
    const map = new Map<string, (typeof snapshots)[number]>();
    for (const s of snapshots) map.set(s.workerId, s);
    return map;
  }, [snapshots]);

  const onlineCount = workers.filter((w) => w.status === WorkerStatus.ONLINE).length;

  if (error) {
    return (
      <div className="h-full flex items-center justify-center p-6 bg-background">
        <QueryError error={error} onRetry={refetch} className="max-w-md" />
      </div>
    );
  }

  if (isLoading) {
    return (
      <div className="p-6">
        <div className="mb-6">
          <div className="h-8 w-48 rounded bg-muted animate-pulse" />
          <div className="h-5 w-64 rounded bg-muted/50 animate-pulse mt-2" />
        </div>
        <div className="grid gap-4">
          {[...Array(3)].map((_, i) => (
            <div key={i} className="border border-border p-4 h-40 rounded-lg">
              <div className="w-full h-full animate-pulse space-y-3">
                <div className="h-6 w-64 rounded bg-muted" />
                <div className="h-4 w-96 rounded bg-muted/50" />
                <div className="grid grid-cols-6 gap-4">
                  <div className="h-8 rounded bg-muted/30" />
                  <div className="h-8 rounded bg-muted/30" />
                  <div className="h-8 rounded bg-muted/30" />
                  <div className="h-8 rounded bg-muted/30" />
                  <div className="h-8 rounded bg-muted/30" />
                  <div className="h-8 rounded bg-muted/30" />
                </div>
              </div>
            </div>
          ))}
        </div>
      </div>
    );
  }

  return (
    <div className="p-6">
      <div className="mb-6">
        <h1 className="text-xl font-medium">Workers</h1>
        <p className="text-sm text-muted-foreground">
          {onlineCount} online, {workers.length} total
        </p>
      </div>

      <div className="grid gap-4">
        {workers.map((worker) => {
          const snap = telemetryByWorkerId.get(worker.workerId);
          const extra = decodeJsonBytes<WorkerExtra>(snap?.extraJson);
          const hostname = worker.hostname || extra?.hostname || "unknown";
          const subscribed = snap?.subscribed ? "subscribed" : "not subscribed";

          return (
            <Link
              key={worker.workerId}
              to="/$namespaceId/workers/$id"
              params={{ namespaceId: namespace, id: worker.workerId }}
              className="border border-border p-4 hover:bg-muted/30 transition-colors"
            >
              <div className="flex items-start justify-between mb-3">
                <div>
                  <div className="flex items-center gap-2 mb-1">
                    <span className="font-mono text-sm font-medium">{hostname}</span>
                    <Badge variant={getStatusVariant(worker.status)}>
                      {WorkerStatusLabel[worker.status]}
                    </Badge>
                    {snap && (
                      <Badge variant={snap.subscribed ? "default" : "outline"}>
                        {snap.signalBackend}:{subscribed}
                      </Badge>
                    )}
                  </div>
                  <p className="font-mono text-xs text-muted-foreground">{worker.workerId}</p>
                </div>
                <div className="text-right text-xs text-muted-foreground space-y-1">
                  <p>Last heartbeat: {formatRelativeTime(worker.lastHeartbeatAt)}</p>
                  {snap?.lastClaimAt && <p>Last claim: {formatRelativeTime(snap.lastClaimAt)}</p>}
                </div>
              </div>

              <div className="grid grid-cols-8 gap-4 text-sm mb-3">
                <div>
                  <dt className="text-xs text-muted-foreground">Task Queue</dt>
                  <dd className="font-mono">{worker.taskQueue}</dd>
                </div>
                <div>
                  <dt className="text-xs text-muted-foreground">Subscribe</dt>
                  <dd className="font-mono">{snap?.subscriptionState || "—"}</dd>
                </div>
                <div>
                  <dt className="text-xs text-muted-foreground">Last wakeup</dt>
                  <dd className="font-mono">{formatRelativeTime(snap?.lastWakeupAt)}</dd>
                </div>
                <div>
                  <dt className="text-xs text-muted-foreground">In-flight</dt>
                  <dd className="font-mono">
                    {snap ? `${snap.inFlight}/${snap.maxConcurrent || "—"}` : "—"}
                  </dd>
                </div>
                <div>
                  <dt className="text-xs text-muted-foreground">Claim p95</dt>
                  <dd className="font-mono">
                    {snap?.claimRttMsP95 ? `${snap.claimRttMsP95.toFixed(1)}ms` : "—"}
                  </dd>
                </div>
                <div>
                  <dt className="text-xs text-muted-foreground">Claim err Δ</dt>
                  <dd className="font-mono">{snap?.claimErrorDelta ?? "—"}</dd>
                </div>
                <div>
                  <dt className="text-xs text-muted-foreground">Active (auth)</dt>
                  <dd className="font-mono">{snap?.activeWorkflowsAuthoritative ?? "—"}</dd>
                </div>
                <div>
                  <dt className="text-xs text-muted-foreground">Last claim</dt>
                  <dd className="font-mono">{snap?.lastClaimResult || "—"}</dd>
                </div>
              </div>

              {snap?.lastError && (
                <div className="mt-2 text-xs text-destructive font-mono truncate">
                  last_error: {snap.lastError}
                </div>
              )}
            </Link>
          );
        })}
      </div>
    </div>
  );
}
