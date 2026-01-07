import { Badge } from "@/components/ui/badge";
import { QueryError } from "@/components/ui/query-error";
import { useWorkers } from "@/hooks/use-dashboard";
import { WorkerStatus, WorkerStatusLabel } from "@/types";
import { createFileRoute } from "@tanstack/react-router";
import { Timestamp } from "@bufbuild/protobuf";

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
  const date = new Date(timestamp.toDate());
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

export const Route = createFileRoute("/workers")({
  component: WorkersPage,
});

function WorkersPage() {
  const { data: workers, isLoading, error, refetch } = useWorkers();

  if (error) {
    return (
      <div className="h-full flex items-center justify-center p-6 bg-background">
        <QueryError error={error} onRetry={() => refetch()} className="max-w-md" />
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
                <div className="grid grid-cols-4 gap-4">
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

  const onlineCount = workers?.filter((w) => w.status === WorkerStatus.ONLINE).length ?? 0;

  return (
    <div className="p-6">
      <div className="mb-6">
        <h1 className="text-xl font-medium">Workers</h1>
        <p className="text-sm text-muted-foreground">
          {onlineCount} online, {workers?.length ?? 0} total
        </p>
      </div>

      <div className="grid gap-4">
        {workers?.map((worker) => (
          <div key={worker.workerId} className="border border-border p-4">
            <div className="flex items-start justify-between mb-3">
              <div>
                <div className="flex items-center gap-2 mb-1">
                  <span className="font-mono text-sm font-medium">{worker.hostname}</span>
                  <Badge variant={getStatusVariant(worker.status)}>
                    {WorkerStatusLabel[worker.status]}
                  </Badge>
                </div>
                <p className="font-mono text-xs text-muted-foreground">{worker.workerId}</p>
              </div>
              <div className="text-right text-xs text-muted-foreground">
                <p>Last heartbeat: {formatRelativeTime(worker.lastHeartbeatAt)}</p>
              </div>
            </div>

            <div className="grid grid-cols-4 gap-4 text-sm mb-3">
              <div>
                <dt className="text-xs text-muted-foreground">Task Queue</dt>
                <dd className="font-mono">{worker.taskQueue}</dd>
              </div>
              <div>
                <dt className="text-xs text-muted-foreground">Version</dt>
                <dd className="font-mono">{worker.version}</dd>
              </div>
              <div>
                <dt className="text-xs text-muted-foreground">PID</dt>
                <dd className="font-mono">{worker.pid}</dd>
              </div>
              <div>
                <dt className="text-xs text-muted-foreground">Concurrency</dt>
                <dd className="font-mono">{worker.queueConcurrencyLimit ?? "∞"}</dd>
              </div>
            </div>

            <div>
              <dt className="text-xs text-muted-foreground mb-1">Workflow Types</dt>
              <dd className="flex flex-wrap gap-1">
                {worker.workflowTypes.map((type) => (
                  <span key={type} className="px-2 py-0.5 bg-muted text-xs font-mono">
                    {type}
                  </span>
                ))}
              </dd>
            </div>

            {Object.keys(worker.labels).length > 0 && (
              <div className="mt-3 pt-3 border-t border-border">
                <dt className="text-xs text-muted-foreground mb-1">Labels</dt>
                <dd className="flex flex-wrap gap-1">
                  {Object.entries(worker.labels).map(([key, value]) => (
                    <span key={key} className="px-2 py-0.5 border border-border text-xs font-mono">
                      {key}={value}
                    </span>
                  ))}
                </dd>
              </div>
            )}
          </div>
        ))}
      </div>
    </div>
  );
}
