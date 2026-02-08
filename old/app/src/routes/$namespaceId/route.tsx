import { createFileRoute, Outlet } from "@tanstack/react-router";

export const Route = createFileRoute("/$namespaceId")({
  component: RouteComponent,
});

function RouteComponent() {
  return <Outlet />;
}
