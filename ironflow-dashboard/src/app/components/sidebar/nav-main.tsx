import { ChevronRight, type LucideIcon } from "lucide-react";
import { useLocation, Link } from "react-router";
import {
	Collapsible,
	CollapsibleContent,
	CollapsibleTrigger,
} from "@/components/ui/collapsible";
import {
	SidebarGroup,
	SidebarMenu,
	SidebarMenuButton,
	SidebarMenuItem,
	SidebarMenuSub,
	SidebarMenuSubButton,
	SidebarMenuSubItem,
} from "@/components/ui/sidebar";

export interface NavItem {
	title: string;
	url: string;
	icon?: LucideIcon;
	isActive?: boolean;
	exactMatch?: boolean;
	items?: {
		title: string;
		url: string;
		icon?: LucideIcon;
		exactMatch?: boolean;
	}[];
}

interface NavMainProps {
	items: NavItem[];
}

export function NavMain({ items }: NavMainProps) {
	const { pathname } = useLocation();

	const isPathActive = (url: string, exact?: boolean) =>
		exact
			? pathname === url
			: pathname === url || pathname.startsWith(`${url}/`);

	return (
		<SidebarGroup>
			<SidebarMenu>
				{items.map((item) => {
					const isItemActive = isPathActive(item.url, item.exactMatch);
					const hasActiveChild = item.items?.some((sub) =>
						isPathActive(sub.url, sub.exactMatch),
					);
					const isActive = isItemActive || hasActiveChild;

					return (
						<Collapsible
							key={item.title}
							defaultOpen={item.isActive || isActive}
							className="group/collapsible"
						>
							<SidebarMenuItem>
								<CollapsibleTrigger
									render={
										<SidebarMenuButton tooltip={item.title} isActive={isActive}>
											{item.icon && <item.icon />}
											<span>{item.title}</span>
											<ChevronRight className="ml-auto transition-transform duration-200 group-data-[state=open]/collapsible:rotate-90" />
										</SidebarMenuButton>
									}
								/>
								<CollapsibleContent>
									<SidebarMenuSub>
										{item.items?.map((subItem) => {
											const isSubActive = isPathActive(
												subItem.url,
												subItem.exactMatch,
											);

											return (
												<SidebarMenuSubItem key={subItem.title}>
													<SidebarMenuSubButton
														render={<Link to={subItem.url} />}
														isActive={isSubActive}
													>
														{subItem.icon && <subItem.icon />}
														<span>{subItem.title}</span>
													</SidebarMenuSubButton>
												</SidebarMenuSubItem>
											);
										})}
									</SidebarMenuSub>
								</CollapsibleContent>
							</SidebarMenuItem>
						</Collapsible>
					);
				})}
			</SidebarMenu>
		</SidebarGroup>
	);
}
