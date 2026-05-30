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
			<CardContent className="p-4">
				<div className="flex items-center gap-3">
					{Icon && (
						<div
							aria-hidden="true"
							className={cn(
								"flex-shrink-0 rounded-[var(--radius-sm)] p-2",
								iconClassName ?? "bg-muted text-muted-foreground",
							)}
						>
							<Icon className="w-4 h-4" />
						</div>
					)}
					<div className="min-w-0">
						<p className="text-xs text-muted-foreground truncate">{label}</p>
						<p className="text-xl font-semibold tracking-tight truncate tabular-nums">
							{value}
						</p>
					</div>
				</div>
			</CardContent>
		</Card>
	);
}
