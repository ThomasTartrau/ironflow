import {
	BarChart3,
	BookOpen,
	KeyRound,
	LayoutDashboard,
	LockKeyhole,
	Play,
	Settings,
	Users,
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
import { useAppSelector } from "@/app/store";
import { useBranding } from "@/app/lib/branding";

const baseNavItems: NavItem[] = [
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
	{
		title: "Settings",
		url: "/api-keys",
		icon: Settings,
		items: [
			{
				title: "API Keys",
				url: "/api-keys",
				icon: KeyRound,
			},
			{
				title: "Secrets",
				url: "/secrets",
				icon: LockKeyhole,
			},
		],
	},
];

const adminNavItem: NavItem = {
	title: "Administration",
	url: "/users",
	icon: Users,
	items: [
		{
			title: "Users",
			url: "/users",
			icon: Users,
		},
	],
};

export function AppSidebar() {
	const auth = useAppSelector((state) => state.auth);
	const branding = useBranding();
	const isAdmin = auth.status === "authenticated" && auth.user.is_admin;

	const navItems = isAdmin ? [...baseNavItems, adminNavItem] : baseNavItems;

	return (
		<Sidebar collapsible="icon">
			<SidebarHeader className="px-4 py-6">
				<div className="flex items-center gap-2">
					<img
						src={branding.logoUrl}
						alt={branding.name}
						className="w-8 h-8 rounded-lg"
					/>
					<span className="text-sm font-bold truncate group-data-[collapsible=icon]:hidden">
						{branding.name}
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
