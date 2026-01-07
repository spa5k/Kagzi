import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Alert01Icon, RefreshIcon, WifiOff01Icon } from "@hugeicons/core-free-icons";
import { HugeiconsIcon } from "@hugeicons/react";
import { cn } from "@/lib/utils";

interface QueryErrorProps {
  error: Error | null;
  onRetry?: () => void;
  className?: string;
  title?: string;
  description?: string;
}

export function QueryError({ error, onRetry, className, title, description }: QueryErrorProps) {
  const isConnectionError =
    error?.message?.toLowerCase().includes("fetch") ||
    error?.message?.toLowerCase().includes("network") ||
    error?.message?.toLowerCase().includes("connection");

  return (
    <Card className={cn("p-6", className)}>
      <div className="flex flex-col items-center justify-center text-center space-y-4 py-8">
        <div className="flex h-12 w-12 items-center justify-center rounded-full bg-destructive/10">
          <HugeiconsIcon
            icon={isConnectionError ? WifiOff01Icon : Alert01Icon}
            className="h-6 w-6 text-destructive"
          />
        </div>
        <div className="space-y-2">
          <h3 className="font-semibold text-lg">
            {title || (isConnectionError ? "Connection Error" : "Error Loading Data")}
          </h3>
          <p className="text-sm text-muted-foreground max-w-md">
            {description ||
              (isConnectionError
                ? "Unable to connect to Kagzi server. Please check that the server is running at " +
                  import.meta.env.VITE_KAGZI_API_URL
                : error?.message || "An unexpected error occurred")}
          </p>
        </div>
        {onRetry && (
          <Button onClick={onRetry} variant="outline" className="gap-2">
            <HugeiconsIcon icon={RefreshIcon} className="h-4 w-4" />
            Retry
          </Button>
        )}
      </div>
    </Card>
  );
}
