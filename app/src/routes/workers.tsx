import { createFileRoute, useNavigate } from "@tanstack/react-router";
import * as React from "react";

const NAMESPACE_STORAGE_KEY = "kagzi:namespace";

export const Route = createFileRoute("/workers")({
  component: WorkersRedirect,
});

function WorkersRedirect() {
  const navigate = useNavigate();

  React.useEffect(() => {
    const namespaceId = localStorage.getItem(NAMESPACE_STORAGE_KEY) || "default";
    void navigate({
      to: "/$namespaceId/workers",
      params: { namespaceId },
      replace: true,
    });
  }, [navigate]);

  return (
    <div className="p-6">
      <p className="text-sm text-muted-foreground">Redirecting…</p>
    </div>
  );
}
