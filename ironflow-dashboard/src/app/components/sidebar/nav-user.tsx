import { ChevronsUpDown, LogOut, Monitor, Moon, Sun } from "lucide-react";
import type { ReactNode } from "react";
import { useNavigate } from "react-router";
import { Avatar, AvatarFallback } from "@/components/ui/avatar";
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuGroup,
	DropdownMenuItem,
	DropdownMenuLabel,
	DropdownMenuSeparator,
	DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
	SidebarMenu,
	SidebarMenuButton,
	SidebarMenuItem,
	useSidebar,
} from "@/components/ui/sidebar";
import { api } from "@/app/lib/api";
import { useTheme, type ThemeMode } from "@/app/lib/theme";
import { useAppDispatch, useAppSelector } from "@/app/store";
import { logout } from "@/app/store/auth-slice";

function getInitials(username: string): string {
	return username.slice(0, 2).toUpperCase();
}

export function NavUser() {
	const { isMobile } = useSidebar();
	const navigate = useNavigate();
	const dispatch = useAppDispatch();
	const auth = useAppSelector((state) => state.auth);
	const { theme, setTheme } = useTheme();

	if (auth.status !== "authenticated") {
		return null;
	}

	const { user } = auth;

	const handleSignOut = () => {
		api
			.post("/auth/sign-out")
			.catch(() => {})
			.finally(() => {
				dispatch(logout());
				navigate("/sign-in");
			});
	};

	const themeItems: { value: ThemeMode; label: string; icon: ReactNode }[] = [
		{ value: "light", label: "Light", icon: <Sun className="size-4" /> },
		{ value: "dark", label: "Dark", icon: <Moon className="size-4" /> },
		{
			value: "system",
			label: "System",
			icon: <Monitor className="size-4" />,
		},
	];

	return (
		<SidebarMenu>
			<SidebarMenuItem>
				<DropdownMenu>
					<DropdownMenuTrigger
						render={
							<SidebarMenuButton
								size="lg"
								className="data-[state=open]:bg-sidebar-accent data-[state=open]:text-sidebar-accent-foreground"
								aria-label={`User menu for ${user.username}`}
							>
								<Avatar
									className="h-8 w-8 rounded-[var(--radius-sm)]"
									role="img"
									aria-label={user.username}
								>
									<AvatarFallback className="rounded-[var(--radius-sm)] text-xs">
										{getInitials(user.username)}
									</AvatarFallback>
								</Avatar>
								<div className="grid flex-1 text-left text-sm leading-tight">
									<span className="truncate font-medium">{user.username}</span>
									<span className="truncate text-xs text-muted-foreground">
										{user.email}
									</span>
								</div>
								<ChevronsUpDown className="ml-auto size-4" />
							</SidebarMenuButton>
						}
					/>
					<DropdownMenuContent
						side={isMobile ? "bottom" : "right"}
						align="end"
						sideOffset={4}
						className="w-56"
					>
						<DropdownMenuGroup>
							<DropdownMenuLabel className="p-0 font-normal">
								<div className="flex items-center gap-2 px-1 py-1.5 text-left text-sm">
									<Avatar
										className="h-8 w-8 rounded-[var(--radius-sm)]"
										role="img"
										aria-label={user.username}
									>
										<AvatarFallback className="rounded-[var(--radius-sm)] text-xs">
											{getInitials(user.username)}
										</AvatarFallback>
									</Avatar>
									<div className="grid flex-1 text-left text-sm leading-tight">
										<span className="truncate font-semibold">
											{user.username}
										</span>
										<span className="truncate text-xs text-muted-foreground">
											{user.email}
										</span>
									</div>
								</div>
							</DropdownMenuLabel>
						</DropdownMenuGroup>
						<DropdownMenuSeparator />
						<DropdownMenuGroup>
							{themeItems.map((item) => (
								<DropdownMenuItem
									key={item.value}
									onClick={() => setTheme(item.value)}
									className={
										theme === item.value
											? "bg-accent text-accent-foreground"
											: ""
									}
								>
									{item.icon}
									{item.label}
									{theme === item.value && (
										<span className="ml-auto text-xs text-muted-foreground">
											active
										</span>
									)}
								</DropdownMenuItem>
							))}
						</DropdownMenuGroup>
						<DropdownMenuSeparator />
						<DropdownMenuGroup>
							<DropdownMenuItem onClick={handleSignOut}>
								<LogOut className="size-4" />
								Sign out
							</DropdownMenuItem>
						</DropdownMenuGroup>
					</DropdownMenuContent>
				</DropdownMenu>
			</SidebarMenuItem>
		</SidebarMenu>
	);
}
