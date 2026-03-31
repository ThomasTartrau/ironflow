import { ChevronsUpDown, LogOut, User } from "lucide-react";
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

	return (
		<SidebarMenu>
			<SidebarMenuItem>
				<DropdownMenu>
					<DropdownMenuTrigger
						render={
							<SidebarMenuButton
								size="lg"
								className="data-[state=open]:bg-sidebar-accent data-[state=open]:text-sidebar-accent-foreground"
							>
								<Avatar className="h-8 w-8 rounded-lg">
									<AvatarFallback className="rounded-lg text-xs">
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
						className="w-56 rounded-lg"
						side={isMobile ? "bottom" : "right"}
						align="end"
						sideOffset={4}
					>
						<DropdownMenuGroup>
							<DropdownMenuLabel className="p-0 font-normal">
								<div className="flex items-center gap-2 px-1 py-1.5 text-left text-sm">
									<Avatar className="h-8 w-8 rounded-lg">
										<AvatarFallback className="rounded-lg text-xs">
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
							<DropdownMenuItem onClick={() => navigate("/")}>
								<User className="size-4" />
								Dashboard
							</DropdownMenuItem>
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
