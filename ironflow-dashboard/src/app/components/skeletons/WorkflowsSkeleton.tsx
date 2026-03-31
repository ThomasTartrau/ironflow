import { Skeleton } from "@/components/ui/skeleton";
import { WorkflowCardSkeleton } from "./shared";

export function WorkflowsListSkeleton() {
	return (
		<div className="px-4 py-2">
			<div className="mb-4">
				<Skeleton className="h-8 w-32 mb-2" />
				<Skeleton className="h-5 w-72" />
			</div>
			<div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
				{Array.from({ length: 6 }, (_, i) => (
					<WorkflowCardSkeleton key={i} />
				))}
			</div>
		</div>
	);
}
