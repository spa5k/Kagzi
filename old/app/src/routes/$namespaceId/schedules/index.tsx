import { Badge } from "@/components/ui/badge";
import { QueryError } from "@/components/ui/query-error";
import { TableSkeleton } from "@/components/ui/table-skeleton";
import { useSchedules } from "@/hooks/use-dashboard";
import { type Timestamp, timestampDate } from "@bufbuild/protobuf/wkt";
import { Link } from "@tanstack/react-router";
import { createFileRoute } from "@tanstack/react-router";

function formatCron(expr: string): string {
  const parts = expr.split(" ");
  if (parts.length !== 5) return expr;

  if (expr === "0 9 * * *") return "Daily at 9:00 AM";
  if (expr === "0 0 * * 0") return "Weekly on Sunday";
  if (expr.startsWith("*/")) return `Every ${parts[0]?.replace("*/", "")} minutes`;

  return expr;
}

function formatRelativeTime(timestamp: Timestamp | undefined): string {
  if (!timestamp) return "—";
  const date = timestampDate(timestamp);
  const now = new Date();
  const diffMs = now.getTime() - date.getTime();
  const future = diffMs < 0;
  const absDiffMs = Math.abs(diffMs);

  const diffMins = Math.floor(absDiffMs / 60000);
  const diffHours = Math.floor(diffMins / 60);
  const diffDays = Math.floor(diffHours / 24);

  if (diffDays > 0) return future ? `in ${diffDays}d` : `${diffDays}d ago`;
  if (diffHours > 0) return future ? `in ${diffHours}h` : `${diffHours}h ago`;
  if (diffMins > 0) return future ? `in ${diffMins}m` : `${diffMins}m ago`;
  return future ? "soon" : "just now";
}

export const Route = createFileRoute("/$namespaceId/schedules/")({
  component: SchedulesPage,
});

function SchedulesPage() {
  const { data: schedules, isLoading, error, refetch } = useSchedules();

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
        <TableSkeleton />
      </div>
    );
  }

  return (
    <div className="p-6">
      <div className="mb-6">
        <h1 className="text-xl font-medium">Schedules</h1>
        <p className="text-sm text-muted-foreground">
          {schedules?.length ?? 0} configured schedules
        </p>
      </div>

      <div className="border border-border">
        <table className="w-full">
          <thead>
            <tr className="border-b border-border bg-muted/30">
              <th className="text-left p-3 text-xs font-medium text-muted-foreground">Workflow</th>
              <th className="text-left p-3 text-xs font-medium text-muted-foreground">Schedule</th>
              <th className="text-left p-3 text-xs font-medium text-muted-foreground">Queue</th>
              <th className="text-left p-3 text-xs font-medium text-muted-foreground">Status</th>
              <th className="text-left p-3 text-xs font-medium text-muted-foreground">Next Run</th>
              <th className="text-left p-3 text-xs font-medium text-muted-foreground">Last Run</th>
            </tr>
          </thead>
          <tbody>
            {schedules?.map((schedule) => (
              <tr
                key={schedule.scheduleId}
                className="border-b border-border last:border-b-0 hover:bg-muted/20 cursor-pointer transition-colors"
              >
                <td className="p-3">
                  <Link
                    to="/schedules/$id"
                    params={{ id: schedule.scheduleId }}
                    className="font-medium text-sm hover:text-primary transition-colors"
                  >
                    {schedule.workflowType}
                  </Link>
                </td>
                <td className="p-3">
                  <div>
                    <span className="font-mono text-sm">{schedule.cronExpr}</span>
                    <p className="text-xs text-muted-foreground">{formatCron(schedule.cronExpr)}</p>
                  </div>
                </td>
                <td className="p-3">
                  <span className="font-mono text-sm">{schedule.taskQueue}</span>
                </td>
                <td className="p-3">
                  <Badge variant={schedule.enabled ? "default" : "secondary"}>
                    {schedule.enabled ? "Active" : "Paused"}
                  </Badge>
                </td>
                <td className="p-3 font-mono text-sm">{formatRelativeTime(schedule.nextFireAt)}</td>
                <td className="p-3 font-mono text-sm">
                  {formatRelativeTime(schedule.lastFiredAt)}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}
