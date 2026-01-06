import { NamespaceSwitcher } from "@/components/layout/namespace-switcher";
import { NavMain, type NavItem } from "@/components/layout/nav-main";
import { NavUser } from "@/components/layout/nav-user";
import {
  Sidebar,
  SidebarContent,
  SidebarFooter,
  SidebarHeader,
  SidebarRail,
} from "@/components/ui/sidebar";
import { Calendar, Clock1, Cpu, Home } from "@hugeicons/core-free-icons";
import type { IconSvgElement } from "@hugeicons/react";
import type * as React from "react";

const monitorItems: NavItem[] = [
  {
    title: "Overview",
    url: "/",
    icon: Home as IconSvgElement,
  },
  {
    title: "Workflows",
    url: "/workflows",
    icon: Clock1 as IconSvgElement,
  },
  {
    title: "Schedules",
    url: "/schedules",
    icon: Calendar as IconSvgElement,
  },
  {
    title: "Workers",
    url: "/workers",
    icon: Cpu as IconSvgElement,
  },
];

const userData = {
  name: "Admin",
  email: "admin@kagzi.dev",
  avatar: "",
};

export function AppSidebar({ ...props }: React.ComponentProps<typeof Sidebar>) {
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
