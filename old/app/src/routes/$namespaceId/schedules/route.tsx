import { createFileRoute, Outlet } from "@tanstack/react-router";

export const Route = createFileRoute("/$namespaceId/schedules")({
  component: RouteComponent,
});

function RouteComponent() {
  return <Outlet />;
}
