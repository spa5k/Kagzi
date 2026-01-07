import { Card } from "@/components/ui/card";
import { cn } from "@/lib/utils";

export function TableSkeleton({ className }: { className?: string }) {
  return (
    <Card className={cn("overflow-hidden", className)}>
      <div className="w-full animate-pulse">
        <div className="h-12 border-b border-border bg-muted/50 flex items-center px-4">
          <div className="h-4 w-10 rounded bg-muted" />
          <div className="ml-4 h-4 w-16 rounded bg-muted" />
          <div className="ml-4 h-4 w-24 rounded bg-muted" />
          <div className="ml-auto h-4 w-24 rounded bg-muted" />
        </div>
        {[...Array(5)].map((_, i) => (
          <div
            key={i}
            className="h-14 border-b border-border flex items-center px-4 last:border-b-0"
          >
            <div className="h-4 w-10 rounded bg-muted" />
            <div className="ml-4 h-4 w-20 rounded bg-muted" />
            <div className="ml-4 h-4 w-32 rounded bg-muted" />
            <div className="ml-auto h-4 w-24 rounded bg-muted" />
          </div>
        ))}
      </div>
    </Card>
  );
}
