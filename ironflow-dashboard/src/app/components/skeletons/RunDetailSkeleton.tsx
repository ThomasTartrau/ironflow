import { Skeleton } from "@/components/ui/skeleton";
import { StatCardSkeleton } from "./shared";

function StepRowSkeleton() {
	return (
		<div className="flex items-center gap-4 px-4 py-3 border-b last:border-0">
			<Skeleton className="h-4 w-32" />
			<Skeleton className="h-5 w-12 rounded-full" />
			<Skeleton className="h-5 w-16 rounded-full" />
			<Skeleton className="h-4 w-14" />
			<Skeleton className="h-4 w-14" />
			<Skeleton className="h-4 w-4 ml-auto" />
		</div>
	);
}

export function RunDetailSkeleton() {
	return (
		<div className="px-4 py-2">
			<div className="mb-4">
				<div className="flex justify-between items-center">
					<Skeleton className="h-8 w-40 mb-2" />
					<div className="flex items-center gap-2">
						<Skeleton className="h-5 w-16 rounded-full" />
						<Skeleton className="h-5 w-20 rounded-full" />
					</div>
				</div>
				<Skeleton className="h-5 w-72" />
			</div>
			<div className="space-y-6">
				<Skeleton className="h-8 w-28 rounded-md" />

				<div className="grid grid-cols-2 md:grid-cols-3 lg:grid-cols-5 gap-3">
					{Array.from({ length: 5 }, (_, i) => (
						<StatCardSkeleton key={i} />
					))}
				</div>

				<div className="space-y-3">
					<Skeleton className="h-5 w-24" />
					<div className="rounded-lg border">
						{Array.from({ length: 4 }, (_, i) => (
							<StepRowSkeleton key={i} />
						))}
					</div>
				</div>
			</div>
		</div>
	);
}
