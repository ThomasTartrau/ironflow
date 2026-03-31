import { Link } from "react-router";
import type { RunResponse } from "@/app/lib/types";
import { StatusBadge } from "@/app/components/StatusBadge";
import { TriggerBadge } from "@/app/components/TriggerBadge";
import { TimeAgo } from "@/app/components/TimeAgo";
import {
	Table,
	TableBody,
	TableCell,
	TableHead,
	TableHeader,
	TableRow,
} from "@/components/ui/table";

import { formatDuration, formatCost } from "@/app/lib/format";

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
			{runs.length === 0 ? (
				<div className="text-center py-12 text-muted-foreground border rounded-lg bg-muted/20">
					No runs yet. Trigger a workflow to see results here.
				</div>
			) : (
				<div className="rounded-lg border">
					<Table>
						<TableHeader>
							<TableRow>
								<TableHead>Status</TableHead>
								<TableHead>Workflow</TableHead>
								<TableHead>Trigger</TableHead>
								<TableHead>Duration</TableHead>
								<TableHead>Cost</TableHead>
								<TableHead>Started</TableHead>
							</TableRow>
						</TableHeader>
						<TableBody>
							{runs.map((run) => (
								<TableRow key={run.id} className="hover:bg-muted/50">
									<TableCell>
										<StatusBadge status={run.status} />
									</TableCell>
									<TableCell>
										<Link to={`/runs/${run.id}`} className="hover:underline">
											{run.workflow_name}
										</Link>
									</TableCell>
									<TableCell>
										<TriggerBadge trigger={run.trigger} />
									</TableCell>
									<TableCell>{formatDuration(run.duration_ms)}</TableCell>
									<TableCell>{formatCost(run.cost_usd)}</TableCell>
									<TableCell>
										<TimeAgo date={run.started_at || run.created_at} />
									</TableCell>
								</TableRow>
							))}
						</TableBody>
					</Table>
				</div>
			)}
		</div>
	);
}
