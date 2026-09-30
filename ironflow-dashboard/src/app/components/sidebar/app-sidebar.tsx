import {
	BarChart3,
	BookOpen,
	CalendarClock,
	Gauge,
	KeyRound,
	LayoutDashboard,
	LockKeyhole,
	Package,
	Play,
	ScrollText,
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
import { useBranding, useBrandLogo } from "@/app/lib/branding";

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
			{
				title: "Templates",
				url: "/templates",
				icon: Package,
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
			{
				title: "Schedules",
				url: "/schedules",
				icon: CalendarClock,
			},
		],
	},
];

/** Settings entry only admins see, right after "Secrets". */
const accountsNavItem = {
	title: "Accounts",
	url: "/accounts",
	icon: Gauge,
};

/** Insert the admin-only "Accounts" entry after "Secrets" in the Settings group. */
function withAccounts(items: NavItem[]): NavItem[] {
	return items.map((item) => {
		if (item.title !== "Settings" || !item.items) return item;
		const index = item.items.findIndex((sub) => sub.url === "/secrets");
		const subItems = [...item.items];
		subItems.splice(index + 1, 0, accountsNavItem);
		return { ...item, items: subItems };
	});
}

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
		{
			title: "Audit Logs",
			url: "/audit-logs",
			icon: ScrollText,
		},
	],
};

export function AppSidebar() {
	const auth = useAppSelector((state) => state.auth);
	const branding = useBranding();
	const logoUrl = useBrandLogo();
	const isAdmin = auth.status === "authenticated" && auth.user.is_admin;

	const navItems = isAdmin
		? [...withAccounts(baseNavItems), adminNavItem]
		: baseNavItems;

	return (
		<Sidebar collapsible="icon">
			<SidebarHeader className="px-3 py-4">
				<div className="flex items-center gap-2">
					<img
						src={logoUrl}
						alt={branding.name}
						className="w-7 h-7 rounded-[var(--radius-sm)] shrink-0"
						width={28}
						height={28}
					/>
					<span className="text-sm font-semibold tracking-tight truncate group-data-[collapsible=icon]:hidden">
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
