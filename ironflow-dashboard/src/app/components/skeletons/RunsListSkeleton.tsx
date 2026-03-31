import { Skeleton } from "@/components/ui/skeleton";
import { TableRowSkeleton } from "./shared";

export function RunsListSkeleton() {
	return (
		<div className="px-4 py-2">
			<div className="mb-4">
				<div className="flex justify-between items-center">
					<Skeleton className="h-8 w-24 mb-2" />
					<Skeleton className="h-9 w-24 rounded-md" />
				</div>
				<Skeleton className="h-5 w-64" />
			</div>
			<div className="space-y-6">
				<div className="mt-6 flex flex-col gap-4 md:flex-row md:items-end">
					<div className="flex-1 space-y-1">
						<Skeleton className="h-4 w-24" />
						<Skeleton className="h-9 w-full rounded-md" />
					</div>
					<div className="w-full md:w-48 space-y-1">
						<Skeleton className="h-4 w-16" />
						<Skeleton className="h-9 w-full rounded-md" />
					</div>
					<Skeleton className="h-9 w-16 rounded-md" />
				</div>

				<div className="rounded-lg border">
					{Array.from({ length: 10 }, (_, i) => (
						<TableRowSkeleton key={i} />
					))}
				</div>

				<div className="flex items-center justify-between">
					<Skeleton className="h-4 w-40" />
					<div className="flex gap-1">
						{Array.from({ length: 5 }, (_, i) => (
							<Skeleton key={i} className="size-8 rounded-md" />
						))}
					</div>
				</div>
			</div>
		</div>
	);
}
