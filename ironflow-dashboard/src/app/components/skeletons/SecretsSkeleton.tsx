import { Skeleton } from "@/components/ui/skeleton";

export function SecretsListSkeleton() {
	return (
		<div className="px-4 py-2">
			<div className="mb-4">
				<Skeleton className="h-8 w-32 mb-2" />
				<Skeleton className="h-5 w-72" />
			</div>
			<div className="rounded-lg border">
				<div className="p-4 space-y-4">
					{Array.from({ length: 4 }, (_, i) => (
						<div key={i} className="flex items-center justify-between">
							<div className="space-y-2">
								<Skeleton className="h-5 w-40" />
								<Skeleton className="h-4 w-24" />
							</div>
							<Skeleton className="h-8 w-20" />
						</div>
					))}
				</div>
			</div>
		</div>
	);
}
