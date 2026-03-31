import type { LucideIcon } from "lucide-react";
import { Card, CardContent } from "@/components/ui/card";
import { cn } from "@/lib/utils";

interface StatCardProps {
	label: string;
	value: React.ReactNode;
	icon?: LucideIcon;
	iconClassName?: string;
	className?: string;
}

export function StatCard({
	label,
	value,
	icon: Icon,
	iconClassName,
	className,
}: StatCardProps) {
	return (
		<Card className={cn("py-0 gap-0", className)}>
			<CardContent className="p-5">
				<div className="flex items-center gap-4">
					{Icon && (
						<div
							className={cn(
								"flex-shrink-0 rounded-lg p-2.5",
								iconClassName ?? "bg-muted text-muted-foreground",
							)}
						>
							<Icon className="w-5 h-5" />
						</div>
					)}
					<div className="min-w-0">
						<p className="text-sm text-muted-foreground truncate">{label}</p>
						<p className="text-2xl font-semibold tracking-tight truncate">
							{value}
						</p>
					</div>
				</div>
			</CardContent>
		</Card>
	);
}
