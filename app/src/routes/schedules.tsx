import { Badge } from "@/components/ui/badge";
import { mockSchedules } from "@/lib/mock-data";
import { createFileRoute } from "@tanstack/react-router";

function formatCron(expr: string): string {
  const parts = expr.split(" ");
  if (parts.length !== 5) return expr;

  if (expr === "0 9 * * *") return "Daily at 9:00 AM";
  if (expr === "0 0 * * 0") return "Weekly on Sunday";
  if (expr.startsWith("*/")) return `Every ${parts[0]?.replace("*/", "")} minutes`;

  return expr;
}

function formatRelativeTime(dateString: string | null): string {
  if (!dateString) return "—";
  const date = new Date(dateString);
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

export const Route = createFileRoute("/schedules")({
  component: SchedulesPage,
});

function SchedulesPage() {
  const schedules = mockSchedules;

  return (
    <div className="p-6">
      <div className="mb-6">
        <h1 className="text-xl font-medium">Schedules</h1>
        <p className="text-sm text-muted-foreground">{schedules.length} configured schedules</p>
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
            {schedules.map((schedule) => (
              <tr
                key={schedule.scheduleId}
                className="border-b border-border last:border-b-0 hover:bg-muted/20"
              >
                <td className="p-3">
                  <span className="font-medium text-sm">{schedule.workflowType}</span>
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
