import { Badge } from "@/components/ui/badge";
import { QueryError } from "@/components/ui/query-error";
import {
  useGetWorkerTelemetryState,
  useListWorkerTelemetryEvents,
  useListWorkers,
} from "@/lib/api-queries";
import { decodeJsonBytes } from "@/lib/utils";
import { TelemetryLevel, TelemetryLevelLabel, WorkerStatus, WorkerStatusLabel } from "@/types";
import { createFileRoute, useParams } from "@tanstack/react-router";
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

type WorkerExtra = {
  hostname?: string | null;
  pid?: number;
  version?: string | null;
  labels?: Record<string, string>;
  workflow_types?: string[];
};

export const Route = createFileRoute("/$namespaceId/workers/$id")({
  component: WorkerDetailPage,
});

function WorkerDetailPage() {
  const params = useParams({ from: "/$namespaceId/workers/$id" });
  const namespace = params.namespaceId;
  const workerId = params.id;

  // Admin data currently only exposed via list API; fetch list and pick.
  const workersQuery = useListWorkers(namespace);
  const telemetryQuery = useGetWorkerTelemetryState(workerId);
  const eventsQuery = useListWorkerTelemetryEvents(namespace, workerId);

  const error = workersQuery.error || telemetryQuery.error || eventsQuery.error;
  const isLoading = workersQuery.isLoading || telemetryQuery.isLoading || eventsQuery.isLoading;
  const refetch = () => {
    void workersQuery.refetch();
    void telemetryQuery.refetch();
    void eventsQuery.refetch();
  };

  const worker = (workersQuery.data?.workersList ?? []).find((w) => w.workerId === workerId);
  const snap = telemetryQuery.data?.snapshot;
  const extra = decodeJsonBytes<WorkerExtra>(snap?.extraJson);
  const hostname = worker?.hostname || extra?.hostname || "unknown";

  const events = eventsQuery.data?.events ?? [];

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
        <div className="h-5 w-96 rounded bg-muted/50 animate-pulse" />
        <div className="h-32 rounded bg-muted/30 animate-pulse" />
      </div>
    );
  }

  if (!snap) {
    return (
      <div className="p-6">
        <p className="text-sm text-muted-foreground">
          No telemetry snapshot found for this worker.
        </p>
      </div>
    );
  }

  return (
    <div className="p-6 space-y-6">
      <div className="flex items-start justify-between">
        <div>
          <div className="flex items-center gap-2">
            <h1 className="text-xl font-medium font-mono">{hostname}</h1>
            {worker && (
              <Badge variant={getStatusVariant(worker.status)}>
                {WorkerStatusLabel[worker.status]}
              </Badge>
            )}
            <Badge variant={snap.subscribed ? "default" : "outline"}>
              {snap.signalBackend}:{snap.subscribed ? "subscribed" : "not subscribed"}
            </Badge>
          </div>
          <div className="mt-1 font-mono text-xs text-muted-foreground">{workerId}</div>
        </div>
        <div className="text-right text-xs text-muted-foreground space-y-1">
          {worker?.lastHeartbeatAt && (
            <p>Last heartbeat: {formatRelativeTime(worker.lastHeartbeatAt)}</p>
          )}
          {snap.lastClaimAt && <p>Last claim: {formatRelativeTime(snap.lastClaimAt)}</p>}
          {snap.lastWakeupAt && <p>Last wakeup: {formatRelativeTime(snap.lastWakeupAt)}</p>}
        </div>
      </div>

      <div className="border border-border p-4 rounded-md">
        <div className="grid grid-cols-6 gap-4 text-sm">
          <div>
            <dt className="text-xs text-muted-foreground">Task Queue</dt>
            <dd className="font-mono">{snap.taskQueue}</dd>
          </div>
          <div>
            <dt className="text-xs text-muted-foreground">Subscribe</dt>
            <dd className="font-mono">
              {snap.signalBackend}:
              {snap.subscriptionState || (snap.subscribed ? "subscribed" : "not subscribed")}
            </dd>
          </div>
          <div>
            <dt className="text-xs text-muted-foreground">In-flight</dt>
            <dd className="font-mono">
              {snap.inFlight} / {snap.maxConcurrent || "—"}
            </dd>
          </div>
          <div>
            <dt className="text-xs text-muted-foreground">Claim p95</dt>
            <dd className="font-mono">
              {snap.claimRttMsP95 ? `${snap.claimRttMsP95.toFixed(1)}ms` : "—"}
            </dd>
          </div>
          <div>
            <dt className="text-xs text-muted-foreground">Active (auth)</dt>
            <dd className="font-mono">{snap.activeWorkflowsAuthoritative}</dd>
          </div>
          <div>
            <dt className="text-xs text-muted-foreground">Last claim</dt>
            <dd className="font-mono">{snap.lastClaimResult || "—"}</dd>
          </div>
        </div>

        <div className="mt-4 grid grid-cols-6 gap-4 text-sm">
          <div>
            <dt className="text-xs text-muted-foreground">Claim attempts Δ</dt>
            <dd className="font-mono">{snap.claimAttemptsDelta}</dd>
          </div>
          <div>
            <dt className="text-xs text-muted-foreground">Claim ok Δ</dt>
            <dd className="font-mono">{snap.claimSuccessDelta}</dd>
          </div>
          <div>
            <dt className="text-xs text-muted-foreground">No task Δ</dt>
            <dd className="font-mono">{snap.claimNoTaskDelta}</dd>
          </div>
          <div>
            <dt className="text-xs text-muted-foreground">Errors Δ</dt>
            <dd className="font-mono">{snap.claimErrorDelta}</dd>
          </div>
          <div>
            <dt className="text-xs text-muted-foreground">RTT avg</dt>
            <dd className="font-mono">
              {snap.claimRttMsAvg ? `${snap.claimRttMsAvg.toFixed(1)}ms` : "—"}
            </dd>
          </div>
          <div>
            <dt className="text-xs text-muted-foreground">RTT max</dt>
            <dd className="font-mono">
              {snap.claimRttMsMax ? `${snap.claimRttMsMax.toFixed(1)}ms` : "—"}
            </dd>
          </div>
        </div>

        {(snap.lastError || snap.lastClaimError) && (
          <div className="mt-4 grid grid-cols-2 gap-4 text-xs">
            <div className="font-mono text-destructive truncate">
              last_error: {snap.lastError || "—"}
            </div>
            <div className="font-mono text-destructive truncate">
              last_claim_error: {snap.lastClaimError || "—"}
            </div>
          </div>
        )}
      </div>

      {extra?.workflow_types && extra.workflow_types.length > 0 && (
        <div>
          <div className="text-sm font-medium mb-2">Workflow Types</div>
          <div className="flex flex-wrap gap-1">
            {extra.workflow_types.map((t) => (
              <span key={t} className="px-2 py-0.5 bg-muted text-xs font-mono">
                {t}
              </span>
            ))}
          </div>
        </div>
      )}

      <div>
        <div className="text-sm font-medium mb-2">Recent Events</div>
        {events.length === 0 ? (
          <p className="text-xs text-muted-foreground">No events recorded.</p>
        ) : (
          <div className="border border-border rounded-md divide-y divide-border">
            {events.slice(0, 100).map((e, idx) => (
              <div key={`${e.occurredAt?.seconds ?? 0}-${idx}`} className="p-3 flex gap-3">
                <div className="w-28 text-xs text-muted-foreground font-mono shrink-0">
                  {e.occurredAt ? formatRelativeTime(e.occurredAt) : "—"}
                </div>
                <div className="shrink-0">
                  <Badge variant={getLevelVariant(e.level)}>{TelemetryLevelLabel[e.level]}</Badge>
                </div>
                <div className="min-w-0">
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
