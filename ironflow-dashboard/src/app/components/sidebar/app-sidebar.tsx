import {
	BarChart3,
	BookOpen,
	LayoutDashboard,
	Play,
	Workflow,
} from "lucide-react";
import {
	Sidebar,
	SidebarContent,
	SidebarFooter,
	SidebarHeader,
	SidebarRail,
} from "@/components/ui/sidebar";
import { NavMain, type NavItem } from "./nav-main";
import { NavUser } from "./nav-user";

const navItems: NavItem[] = [
	{
		title: "Overview",
		url: "/",
		icon: LayoutDashboard,
		exactMatch: true,
		items: [
			{
				title: "Dashboard",
				url: "/",
				exactMatch: true,
				icon: BarChart3,
			},
		],
	},
	{
		title: "Workflows",
		url: "/workflows",
		icon: Workflow,
		items: [
			{
				title: "All Workflows",
				url: "/workflows",
				icon: BookOpen,
			},
			{
				title: "All Runs",
				url: "/runs",
				icon: Play,
			},
		],
	},
];

export function AppSidebar() {
	return (
		<Sidebar collapsible="icon">
			<SidebarHeader className="px-4 py-6">
				<div className="flex items-center gap-2">
					<img src="/logo.svg" alt="ironflow" className="w-8 h-8 rounded-lg" />
					<span className="text-sm font-bold truncate group-data-[collapsible=icon]:hidden">
						ironflow
					</span>
				</div>
			</SidebarHeader>
			<SidebarContent>
				<NavMain items={navItems} />
			</SidebarContent>
			<SidebarFooter>
				<NavUser />
			</SidebarFooter>
			<SidebarRail />
		</Sidebar>
	);
}
