import { NamespaceSwitcher } from "@/components/layout/namespace-switcher";
import { type NavItem, NavMain } from "@/components/layout/nav-main";
import { NavUser } from "@/components/layout/nav-user";
import {
  Sidebar,
  SidebarContent,
  SidebarFooter,
  SidebarHeader,
  SidebarRail,
} from "@/components/ui/sidebar";
import { Calendar, Clock1, Cpu, Home } from "@hugeicons/core-free-icons";
import { useParams } from "@tanstack/react-router";
import type * as React from "react";

const userData = {
  name: "Admin",
  email: "admin@kagzi.dev",
  avatar: "",
};

export function AppSidebar({ ...props }: React.ComponentProps<typeof Sidebar>) {
  const params = useParams({ strict: false });
  const namespaceId = (params as { namespaceId?: string }).namespaceId || "default";

  const monitorItems: NavItem[] = [
    {
      title: "Overview",
      url: `/${namespaceId}`,
      icon: Home,
    },
    {
      title: "Workflows",
      url: `/${namespaceId}/workflows`,
      icon: Clock1,
    },
    {
      title: "Schedules",
      url: `/${namespaceId}/schedules`,
      icon: Calendar,
    },
    {
      title: "Workers",
      url: "/workers",
      icon: Cpu,
    },
  ];

  return (
    <Sidebar collapsible="icon" {...props}>
      <SidebarHeader>
        <NamespaceSwitcher />
      </SidebarHeader>
      <SidebarContent>
        <NavMain items={monitorItems} label="Monitor" />
      </SidebarContent>
      <SidebarFooter>
        <NavUser user={userData} />
      </SidebarFooter>
      <SidebarRail />
    </Sidebar>
  );
}
