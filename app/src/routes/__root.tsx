import { AppSidebar } from "@/components/layout/app-sidebar";
import {
  Breadcrumb,
  BreadcrumbItem,
  BreadcrumbLink,
  BreadcrumbList,
  BreadcrumbPage,
  BreadcrumbSeparator,
} from "@/components/ui/breadcrumb";
import { Separator } from "@/components/ui/separator";
import { SidebarInset, SidebarProvider, SidebarTrigger } from "@/components/ui/sidebar";
import { createRootRoute, Outlet, useRouterState } from "@tanstack/react-router";
import { TanStackRouterDevtools } from "@tanstack/react-router-devtools";

const sectionLabels: Record<string, string> = {
  workflows: "Workflows",
  schedules: "Schedules",
  workers: "Workers",
  queues: "Queues",
  events: "Events",
  server: "Server",
  namespaces: "Namespaces",
};

function getCurrentLabel(pathname: string): string {
  const parts = pathname.split("/").filter(Boolean);
  if (parts.length === 0) return "Overview";

  // Top-level routes: /namespaces, /workers (legacy), etc.
  const knownTopLevel = new Set(["namespaces", "workers"]);
  const isTopLevel = parts.length >= 1 && knownTopLevel.has(parts[0] ?? "");

  // Namespace root like /default or /default/
  if (!isTopLevel && parts.length === 1) return "Overview";

  const section = isTopLevel ? parts[0] : parts[1] || parts[0];
  return sectionLabels[section] || section || "Dashboard";
}

function RootLayout() {
  const routerState = useRouterState();
  const pathname = routerState.location.pathname;
  const currentLabel = getCurrentLabel(pathname);
  const isHome = pathname === "/";

  return (
    <SidebarProvider>
      <AppSidebar />
      <SidebarInset>
        <header className="flex h-12 shrink-0 items-center gap-2 border-b border-border">
          <div className="flex items-center gap-2 px-4">
            <SidebarTrigger className="-ml-1" />
            <Separator orientation="vertical" className="mr-2 h-4" />
            <Breadcrumb>
              <BreadcrumbList>
                {!isHome && (
                  <>
                    <BreadcrumbItem className="hidden md:block">
                      <BreadcrumbLink href="/">Dashboard</BreadcrumbLink>
                    </BreadcrumbItem>
                    <BreadcrumbSeparator className="hidden md:block" />
                  </>
                )}
                <BreadcrumbItem>
                  <BreadcrumbPage>{currentLabel}</BreadcrumbPage>
                </BreadcrumbItem>
              </BreadcrumbList>
            </Breadcrumb>
          </div>
        </header>
        <div className="flex flex-1 flex-col overflow-hidden">
          <Outlet />
        </div>
        <TanStackRouterDevtools position="bottom-right" />
      </SidebarInset>
    </SidebarProvider>
  );
}

export const Route = createRootRoute({ component: RootLayout });
