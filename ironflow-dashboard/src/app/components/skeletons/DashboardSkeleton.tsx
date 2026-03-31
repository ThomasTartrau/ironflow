import { Skeleton } from "@/components/ui/skeleton";
import { StatCardSkeleton, TableRowSkeleton } from "./shared";

export function DashboardSkeleton() {
	return (
		<div className="px-4 py-2">
			<div className="mb-4">
				<Skeleton className="h-8 w-40 mb-2" />
				<Skeleton className="h-5 w-72" />
			</div>
			<div className="space-y-6">
				<div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-4">
					{Array.from({ length: 4 }, (_, i) => (
						<StatCardSkeleton key={i} />
					))}
				</div>
				<div className="mt-8">
					<div className="flex items-center justify-between mb-4">
						<Skeleton className="h-7 w-32" />
						<Skeleton className="h-4 w-16" />
					</div>
					<div className="rounded-lg border">
						{Array.from({ length: 5 }, (_, i) => (
							<TableRowSkeleton key={i} />
						))}
					</div>
				</div>
			</div>
		</div>
	);
}
