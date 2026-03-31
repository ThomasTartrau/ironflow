import { Skeleton } from "@/components/ui/skeleton";
import { Card, CardContent } from "@/components/ui/card";

export function StatCardSkeleton() {
	return (
		<Card className="py-0 gap-0">
			<CardContent className="p-4">
				<div className="flex items-center gap-3">
					<Skeleton className="size-8 rounded-md flex-shrink-0" />
					<div className="min-w-0 space-y-1.5 flex-1">
						<Skeleton className="h-3 w-16" />
						<Skeleton className="h-5 w-20" />
					</div>
				</div>
			</CardContent>
		</Card>
	);
}

export function TableRowSkeleton() {
	return (
		<div className="flex items-center gap-4 px-4 py-3 border-b last:border-0">
			<Skeleton className="h-5 w-16 rounded-full" />
			<Skeleton className="h-4 w-28" />
			<Skeleton className="h-5 w-14 rounded-full" />
			<Skeleton className="h-4 w-16" />
			<Skeleton className="h-4 w-14" />
			<Skeleton className="h-4 w-20 ml-auto" />
		</div>
	);
}

export function WorkflowCardSkeleton() {
	return (
		<div className="flex flex-col gap-3 rounded-lg border bg-card p-5 shadow-sm">
			<div className="flex items-center gap-3">
				<Skeleton className="size-10 rounded-lg flex-shrink-0" />
				<Skeleton className="h-5 w-32" />
			</div>
			<Skeleton className="h-4 w-full" />
			<Skeleton className="h-4 w-2/3" />
			<Skeleton className="h-3 w-20 mt-auto" />
		</div>
	);
}
