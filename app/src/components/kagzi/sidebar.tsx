import { NavMain } from "@/components/nav-main";
import { NavProjects } from "@/components/nav-projects";
import { Sidebar, SidebarContent, SidebarFooter, SidebarHeader, SidebarRail } from "@/components/ui/sidebar";
import {
  Activity01Icon,
  Box,
  DashboardBrowsingFreeIcons,
  Dna01Icon,
  ListTodo,
  Server,
  Settings01Icon,
} from "@hugeicons/core-free-icons";
import { HugeiconsIcon } from "@hugeicons/react";

const data = {
  navMain: [
    {
      title: "Overview",
      url: "/",
      icon: DashboardBrowsingFreeIcons,
      isActive: true,
    },
    {
      title: "Workflows",
      url: "/workflows",
      icon: ListTodo,
      isActive: false,
    },
    {
      title: "Executions",
      url: "/executions",
      icon: Activity01Icon,
      isActive: false,
    },
    {
      title: "Workers",
      url: "/workers",
      icon: Server,
      isActive: false,
    },
    {
      title: "Schedules",
      url: "/schedules",
      icon: Box,
      isActive: false,
    },
    {
      title: "Settings",
      url: "/settings",
      icon: Settings01Icon,
      isActive: false,
    },
  ],
  projects: [],
};

export function AppSidebar({ ...props }: React.ComponentProps<typeof Sidebar>) {
  return (
    <Sidebar collapsible="icon" {...props}>
      <SidebarHeader>
        <div className="flex items-center gap-2 font-mono font-bold text-xl tracking-tighter px-2">
          <HugeiconsIcon icon={Dna01Icon} strokeWidth={2} className="w-6 h-6 text-primary" />
          <span>KAGZI</span>
        </div>
      </SidebarHeader>
      <SidebarContent>
        <NavMain items={data.navMain} />
        <NavProjects projects={data.projects} />
      </SidebarContent>
      <SidebarFooter>
        <div className="px-2 text-xs text-muted-foreground font-mono">
          v0.1.0-alpha
        </div>
      </SidebarFooter>
      <SidebarRail />
    </Sidebar>
  );
}
