import { Skeleton } from "@/components/ui/skeleton";
import { TableRowSkeleton } from "./shared";

export function WorkflowDetailSkeleton() {
	return (
		<div className="px-4 py-2">
			<div className="mb-4">
				<div className="flex justify-between items-center">
					<Skeleton className="h-8 w-40 mb-2" />
					<Skeleton className="h-9 w-16 rounded-md" />
				</div>
				<Skeleton className="h-5 w-64" />
			</div>
			<div className="space-y-8">
				<Skeleton className="h-8 w-36 rounded-md" />

				<div className="space-y-3">
					<Skeleton className="h-5 w-28" />
					<Skeleton className="h-64 w-full rounded-lg" />
				</div>

				<div className="space-y-3">
					<div className="flex items-center justify-between">
						<Skeleton className="h-5 w-32" />
						<Skeleton className="h-8 w-16" />
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
