import { Link } from "react-router";
import type { RunResponse } from "@/app/lib/types";
import { RunsTable } from "@/app/routes/runs/_components/RunsTable";

interface RecentRunsProps {
	runs: RunResponse[];
}

export function RecentRuns({ runs }: RecentRunsProps) {
	return (
		<div>
			<div className="flex items-center justify-between mb-4">
				<h2 className="text-xs font-medium uppercase tracking-widest text-muted-foreground">
					Recent Runs
				</h2>
				<Link
					to="/runs"
					className="text-sm font-medium text-primary hover:text-primary/80"
				>
					View All
				</Link>
			</div>
			<RunsTable runs={runs} />
		</div>
	);
}
