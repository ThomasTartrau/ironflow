import { Link } from "react-router";
import type { RunResponse } from "@/app/lib/types";
import { RunsTable } from "@/app/routes/runs/_components/RunsTable";

interface RecentRunsProps {
	runs: RunResponse[];
}

export function RecentRuns({ runs }: RecentRunsProps) {
	return (
		<div className="mt-8">
			<div className="flex items-center justify-between mb-4">
				<h2 className="text-2xl font-bold">Recent Runs</h2>
				<Link to="/runs" className="text-sm text-blue-600 hover:text-blue-700">
					View All
				</Link>
			</div>
			<RunsTable runs={runs} />
		</div>
	);
}
