import { Badge } from "@/components/ui/badge";
import { QueryError } from "@/components/ui/query-error";
import { useListServerTelemetryEvents } from "@/lib/api-queries";
import { decodeJsonBytes } from "@/lib/utils";
import { TelemetryLevel, TelemetryLevelLabel } from "@/types";
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

export const Route = createFileRoute("/$namespaceId/events")({
  component: EventsPage,
});

function EventsPage() {
  const params = useParams({ from: "/$namespaceId/events" });
  const namespace = params.namespaceId;

  const query = useListServerTelemetryEvents(namespace);

  if (query.error) {
    return (
      <div className="h-full flex items-center justify-center p-6 bg-background">
        <QueryError error={query.error} onRetry={() => query.refetch()} className="max-w-md" />
      </div>
    );
  }

  if (query.isLoading) {
    return (
      <div className="p-6">
        <div className="mb-6">
          <div className="h-8 w-48 rounded bg-muted animate-pulse" />
          <div className="h-5 w-64 rounded bg-muted/50 animate-pulse mt-2" />
        </div>
        <div className="space-y-2">
          {[...Array(8)].map((_, i) => (
            <div key={i} className="h-12 border border-border rounded bg-muted/20 animate-pulse" />
          ))}
        </div>
      </div>
    );
  }

  const events = query.data?.events ?? [];

  return (
    <div className="p-6 space-y-4">
      <div className="mb-2">
        <h1 className="text-xl font-medium">Events</h1>
        <p className="text-sm text-muted-foreground">{events.length} recent server events</p>
      </div>

      {events.length === 0 ? (
        <p className="text-sm text-muted-foreground">No events recorded.</p>
      ) : (
        <div className="border border-border rounded-md divide-y divide-border">
          {events.slice(0, 200).map((e, idx) => {
            const extra = decodeJsonBytes<Record<string, unknown>>(e.extraJson);
            const hasExtra = extra && Object.keys(extra).length > 0;
            return (
              <div key={`${e.occurredAt?.seconds ?? 0}-${idx}`} className="p-3 flex gap-3">
                <div className="w-28 text-xs text-muted-foreground font-mono shrink-0">
                  {e.occurredAt ? formatRelativeTime(e.occurredAt) : "—"}
                </div>
                <div className="shrink-0">
                  <Badge variant={getLevelVariant(e.level)}>{TelemetryLevelLabel[e.level]}</Badge>
                </div>
                <div className="min-w-0 flex-1">
                  <div className="text-xs font-mono">
                    {e.eventType}{" "}
                    <span className="text-muted-foreground">{e.namespace || namespace}</span>
                  </div>
                  {e.message && (
                    <div className="text-xs text-muted-foreground truncate font-mono">
                      {e.message}
                    </div>
                  )}
                  {hasExtra && (
                    <details className="mt-2">
                      <summary className="text-xs text-muted-foreground cursor-pointer select-none">
                        extra
                      </summary>
                      <pre className="mt-2 text-xs font-mono bg-muted/20 border border-border rounded-md p-3 overflow-auto">
                        {JSON.stringify(extra, null, 2)}
                      </pre>
                    </details>
                  )}
                </div>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}
