import { createFileRoute, Outlet } from "@tanstack/react-router";

export const Route = createFileRoute("/workflows")({
  component: RouteComponent,
});

function RouteComponent() {
  return <Outlet />;
}
