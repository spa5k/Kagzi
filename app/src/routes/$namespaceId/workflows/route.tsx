import { createFileRoute, Outlet } from "@tanstack/react-router";

export const Route = createFileRoute("/$namespaceId/workflows")({
  component: RouteComponent,
});

function RouteComponent() {
  return <Outlet />;
}
