import { Skeleton } from "@/components/ui/skeleton";

export function AuditLogsSkeleton() {
	return (
		<div className="px-4 py-2">
			<div className="mb-4">
				<Skeleton className="h-8 w-36 mb-2" />
				<Skeleton className="h-5 w-80" />
			</div>
			<div className="flex gap-2 mb-4">
				<Skeleton className="h-9 w-40" />
				<Skeleton className="h-9 w-40" />
				<Skeleton className="h-9 w-40" />
			</div>
			<div className="rounded-lg border">
				<div className="p-4 space-y-4">
					{Array.from({ length: 6 }, (_, i) => (
						<div key={i} className="flex items-center gap-4">
							<Skeleton className="h-4 w-28" />
							<Skeleton className="h-4 w-32" />
							<Skeleton className="h-4 w-16" />
							<Skeleton className="h-4 w-16" />
							<Skeleton className="h-4 w-48" />
						</div>
					))}
				</div>
			</div>
		</div>
	);
}
