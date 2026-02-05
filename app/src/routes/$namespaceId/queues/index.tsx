import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { QueryError } from "@/components/ui/query-error";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Sheet,
  SheetClose,
  SheetContent,
  SheetDescription,
  SheetFooter,
  SheetHeader,
  SheetTitle,
  SheetTrigger,
} from "@/components/ui/sheet";
import { useCreateQueue } from "@/hooks/use-grpc-services";
import { useListQueueTelemetryStates, useListQueues } from "@/lib/api-queries";
import { decodeJsonBytes } from "@/lib/utils";
import { createFileRoute, Link, useParams } from "@tanstack/react-router";
import { type Timestamp, timestampDate } from "@bufbuild/protobuf/wkt";
import * as React from "react";

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

export const Route = createFileRoute("/$namespaceId/queues/")({
  component: QueuesPage,
});

function parseLabels(text: string): Record<string, string> {
  const labels: Record<string, string> = {};
  const lines = text.split("\n").map((l) => l.trim());
  for (const line of lines) {
    if (!line) continue;
    const idx = line.indexOf("=");
    if (idx <= 0) continue;
    const key = line.slice(0, idx).trim();
    const value = line.slice(idx + 1).trim();
    if (!key) continue;
    labels[key] = value;
  }
  return labels;
}

function isValidQueueName(name: string): boolean {
  return /^[A-Za-z0-9_-]{1,64}$/.test(name);
}

function QueuesPage() {
  const params = useParams({ from: "/$namespaceId/queues/" });
  const namespace = params.namespaceId;

  const queuesQuery = useListQueues(namespace);
  const telemetryQuery = useListQueueTelemetryStates(namespace);
  const createQueue = useCreateQueue();

  const [isCreateOpen, setIsCreateOpen] = React.useState(false);
  const [queueName, setQueueName] = React.useState("");
  const [displayName, setDisplayName] = React.useState("");
  const [description, setDescription] = React.useState("");
  const [labelsText, setLabelsText] = React.useState("");
  const [extraJsonText, setExtraJsonText] = React.useState("{}");
  const [formError, setFormError] = React.useState<string | null>(null);

  const submitCreate = async () => {
    setFormError(null);
    const name = queueName.trim();
    if (!isValidQueueName(name)) {
      setFormError("Queue name must be 1-64 chars, letters/numbers/_/- only.");
      return;
    }

    let extraBytes = new Uint8Array();
    try {
      const text = extraJsonText.trim();
      if (text) {
        JSON.parse(text);
        extraBytes = new TextEncoder().encode(text);
      }
    } catch {
      setFormError("extra_json must be valid JSON.");
      return;
    }

    try {
      await createQueue.mutateAsync({
        namespace,
        taskQueue: name,
        displayName: displayName.trim() || undefined,
        description: description.trim() || undefined,
        labels: parseLabels(labelsText),
        extraJson: extraBytes,
      });
      setIsCreateOpen(false);
      setQueueName("");
      setDisplayName("");
      setDescription("");
      setLabelsText("");
      setExtraJsonText("{}");
    } catch (e: unknown) {
      const message =
        e instanceof Error
          ? e.message
          : typeof e === "object" && e && "message" in e
            ? String((e as { message: unknown }).message)
            : "Failed to create queue.";
      setFormError(message);
    }
  };

  const error = queuesQuery.error || telemetryQuery.error;
  const isLoading = queuesQuery.isLoading || telemetryQuery.isLoading;

  if (error) {
    return (
      <div className="h-full flex items-center justify-center p-6 bg-background">
        <QueryError
          error={error}
          onRetry={() => {
            void queuesQuery.refetch();
            void telemetryQuery.refetch();
          }}
          className="max-w-md"
        />
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
        <div className="grid gap-3">
          {[...Array(6)].map((_, i) => (
            <div key={i} className="h-16 border border-border rounded bg-muted/20 animate-pulse" />
          ))}
        </div>
      </div>
    );
  }

  const queues = queuesQuery.data?.queues ?? [];
  const states = telemetryQuery.data?.states ?? [];

  const stateByQueue = new Map(states.map((s) => [s.taskQueue, s]));
  const queueByName = new Map(queues.map((q) => [q.taskQueue, q]));

  const allNames = new Set<string>();
  for (const q of queues) allNames.add(q.taskQueue);
  for (const s of states) allNames.add(s.taskQueue);

  const rows = Array.from(allNames)
    .sort((a, b) => a.localeCompare(b))
    .map((name) => ({ name, meta: queueByName.get(name), state: stateByQueue.get(name) }));

  return (
    <div className="p-6">
      <div className="mb-6 flex items-start justify-between gap-4">
        <div>
          <h1 className="text-xl font-medium">Queues</h1>
          <p className="text-sm text-muted-foreground">{rows.length} queues</p>
        </div>

        <Sheet open={isCreateOpen} onOpenChange={setIsCreateOpen}>
          <SheetTrigger render={<Button className="font-mono text-xs uppercase tracking-wider" />}>
            Create queue
          </SheetTrigger>
          <SheetContent className="w-full sm:max-w-lg">
            <SheetHeader>
              <SheetTitle>Create queue</SheetTitle>
              <SheetDescription>
                Queues are metadata-only. Execution still works via implicit creation.
              </SheetDescription>
            </SheetHeader>

            <div className="mt-4 space-y-4">
              <div className="space-y-2">
                <Label htmlFor="queue-name" className="font-mono text-xs uppercase tracking-wider">
                  Queue Name <span className="text-destructive">*</span>
                </Label>
                <Input
                  id="queue-name"
                  value={queueName}
                  onChange={(e) => setQueueName(e.target.value)}
                  placeholder="e.g., high_priority"
                  className="font-mono text-sm"
                />
                <p className="text-[10px] text-muted-foreground font-mono">
                  1–64 chars, letters/numbers/_/-. The default queue is{" "}
                  <span className="font-mono">default</span>.
                </p>
              </div>

              <div className="space-y-2">
                <Label
                  htmlFor="display-name"
                  className="font-mono text-xs uppercase tracking-wider"
                >
                  Display Name
                </Label>
                <Input
                  id="display-name"
                  value={displayName}
                  onChange={(e) => setDisplayName(e.target.value)}
                  placeholder="Optional"
                  className="font-mono text-sm"
                />
              </div>

              <div className="space-y-2">
                <Label htmlFor="description" className="font-mono text-xs uppercase tracking-wider">
                  Description
                </Label>
                <textarea
                  id="description"
                  value={description}
                  onChange={(e) => setDescription(e.target.value)}
                  className="flex min-h-[80px] w-full rounded-md border border-input bg-background px-3 py-2 text-sm ring-offset-background placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-50 font-mono"
                  placeholder="Optional"
                  spellCheck={false}
                />
              </div>

              <div className="space-y-2">
                <Label htmlFor="labels" className="font-mono text-xs uppercase tracking-wider">
                  Labels
                </Label>
                <textarea
                  id="labels"
                  value={labelsText}
                  onChange={(e) => setLabelsText(e.target.value)}
                  className="flex min-h-[80px] w-full rounded-md border border-input bg-background px-3 py-2 text-sm ring-offset-background placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-50 font-mono"
                  placeholder={"team=payments\nlane=cpu"}
                  spellCheck={false}
                />
                <p className="text-[10px] text-muted-foreground font-mono">
                  One key=value per line.
                </p>
              </div>

              <div className="space-y-2">
                <Label htmlFor="extra-json" className="font-mono text-xs uppercase tracking-wider">
                  Extra JSON
                </Label>
                <textarea
                  id="extra-json"
                  value={extraJsonText}
                  onChange={(e) => setExtraJsonText(e.target.value)}
                  className="flex min-h-[120px] w-full rounded-md border border-input bg-background px-3 py-2 text-sm ring-offset-background placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-50 font-mono"
                  placeholder='{"notes":"optional"}'
                  spellCheck={false}
                />
              </div>

              {formError && <div className="text-xs font-mono text-destructive">{formError}</div>}
            </div>

            <SheetFooter className="mt-4">
              <SheetClose
                render={
                  <Button
                    variant="outline"
                    className="font-mono text-xs uppercase tracking-wider"
                  />
                }
              >
                Cancel
              </SheetClose>
              <Button
                onClick={submitCreate}
                disabled={createQueue.isPending}
                className="font-mono text-xs uppercase tracking-wider"
              >
                {createQueue.isPending ? "Creating..." : "Create"}
              </Button>
            </SheetFooter>
          </SheetContent>
        </Sheet>
      </div>

      <div className="grid gap-3">
        {rows.map((r) => {
          const s = r.state;
          const q = r.meta;
          const extraTelemetry = decodeJsonBytes<Record<string, unknown>>(s?.extraJson);
          const extraMeta = decodeJsonBytes<Record<string, unknown>>(q?.extraJson);
          const extra = extraTelemetry || extraMeta;
          const hasError = !!s?.lastPublishError;
          const isEnabled = q?.enabled ?? true;
          const updatedAt = s?.updatedAt || q?.updatedAt;
          return (
            <Link
              key={r.name}
              to="/$namespaceId/queues/$taskQueue"
              params={{ namespaceId: namespace, taskQueue: r.name }}
              className="border border-border rounded-md p-4 hover:bg-muted/30 transition-colors"
            >
              <div className="flex items-start justify-between">
                <div>
                  <div className="flex items-center gap-2">
                    <span className="font-mono text-sm font-medium">
                      {q?.displayName || r.name}
                    </span>
                    {!isEnabled && <Badge variant="secondary">disabled</Badge>}
                    {hasError && <Badge variant="destructive">publish errors</Badge>}
                  </div>
                  <div className="text-xs text-muted-foreground mt-1">
                    Updated: {formatRelativeTime(updatedAt)}
                  </div>
                </div>
                <div className="text-right text-xs text-muted-foreground space-y-1">
                  <div>Due notified: {formatRelativeTime(s?.lastDueWorkNotifiedAt)}</div>
                  <div>Last publish ok: {formatRelativeTime(s?.lastPublishOkAt)}</div>
                </div>
              </div>

              <div className="grid grid-cols-6 gap-4 text-sm mt-3">
                <div>
                  <dt className="text-xs text-muted-foreground">Pending</dt>
                  <dd className="font-mono">{s?.pendingCount ?? 0}</dd>
                </div>
                <div>
                  <dt className="text-xs text-muted-foreground">Sleeping</dt>
                  <dd className="font-mono">{s?.sleepingCount ?? 0}</dd>
                </div>
                <div>
                  <dt className="text-xs text-muted-foreground">Running</dt>
                  <dd className="font-mono">{s?.runningCount ?? 0}</dd>
                </div>
                <div>
                  <dt className="text-xs text-muted-foreground">Due</dt>
                  <dd className="font-mono">{s?.dueCount ?? 0}</dd>
                </div>
                <div>
                  <dt className="text-xs text-muted-foreground">Publish attempts</dt>
                  <dd className="font-mono">{s?.publishAttempts ?? 0}</dd>
                </div>
                <div>
                  <dt className="text-xs text-muted-foreground">Publish errors</dt>
                  <dd className="font-mono">{s?.publishErrors ?? 0}</dd>
                </div>
              </div>

              {extra && Object.keys(extra).length > 0 && (
                <div className="mt-2 text-xs text-muted-foreground font-mono truncate">
                  extra: {JSON.stringify(extra)}
                </div>
              )}
            </Link>
          );
        })}
      </div>
    </div>
  );
}
