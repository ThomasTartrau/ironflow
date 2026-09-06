import { Fragment } from "react";
import { Link } from "react-router";
import {
	Breadcrumb as BreadcrumbRoot,
	BreadcrumbItem,
	BreadcrumbLink,
	BreadcrumbList,
	BreadcrumbPage,
	BreadcrumbSeparator,
} from "@/components/ui/breadcrumb";

interface BreadcrumbEntry {
	label: string;
	to?: string;
}

interface AppBreadcrumbProps {
	items: BreadcrumbEntry[];
}

export function Breadcrumb({ items }: AppBreadcrumbProps) {
	return (
		<BreadcrumbRoot>
			<BreadcrumbList>
				{items.map((item, i) => {
					const isLast = i === items.length - 1;
					return (
						<Fragment key={`${i}-${item.label}`}>
							{i > 0 && <BreadcrumbSeparator />}
							<BreadcrumbItem>
								{item.to && !isLast ? (
									<BreadcrumbLink render={<Link to={item.to} />}>
										{item.label}
									</BreadcrumbLink>
								) : (
									<BreadcrumbPage>{item.label}</BreadcrumbPage>
								)}
							</BreadcrumbItem>
						</Fragment>
					);
				})}
			</BreadcrumbList>
		</BreadcrumbRoot>
	);
}
