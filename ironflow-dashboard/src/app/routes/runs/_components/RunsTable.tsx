import { useNavigate } from "react-router";
import type { RunResponse } from "@/app/lib/types";
import { StatusBadge } from "@/app/components/StatusBadge";
import { TriggerBadge } from "@/app/components/TriggerBadge";
import { TimeAgo } from "@/app/components/TimeAgo";
import { RunLabels } from "@/app/components/RunLabels";
import { formatDuration, formatCost } from "@/app/lib/format";
import {
	Table,
	TableBody,
	TableCell,
	TableHead,
	TableHeader,
	TableRow,
} from "@/components/ui/table";

interface RunsTableProps {
	runs: RunResponse[];
}

export function RunsTable({ runs }: RunsTableProps) {
	const navigate = useNavigate();

	const handleRowClick = (runId: string) => {
		navigate(`/runs/${runId}`);
	};

	if (runs.length === 0) {
		return (
			<div className="text-center py-12 text-muted-foreground border rounded-lg bg-muted/20">
				No runs found. Create one to get started.
			</div>
		);
	}

	const hasVersions = runs.some((r) => r.handler_version);

	return (
		<div className="rounded-lg border">
			<Table>
				<TableHeader>
					<TableRow>
						<TableHead>Status</TableHead>
						<TableHead>Workflow</TableHead>
						{hasVersions && <TableHead>Version</TableHead>}
						<TableHead>Trigger</TableHead>
						<TableHead>Labels</TableHead>
						<TableHead>Duration</TableHead>
						<TableHead>Cost</TableHead>
						<TableHead>Started / Scheduled</TableHead>
					</TableRow>
				</TableHeader>
				<TableBody>
					{runs.map((run) => (
						<TableRow
							key={run.id}
							onClick={() => handleRowClick(run.id)}
							className="cursor-pointer hover:bg-muted/50"
						>
							<TableCell>
								<StatusBadge status={run.status} />
							</TableCell>
							<TableCell className="font-medium">{run.workflow_name}</TableCell>
							{hasVersions && (
								<TableCell className="font-mono text-xs text-muted-foreground">
									{run.handler_version || "-"}
								</TableCell>
							)}
							<TableCell>
								<TriggerBadge trigger={run.trigger} />
							</TableCell>
							<TableCell>
								<RunLabels labels={run.labels} />
							</TableCell>
							<TableCell>{formatDuration(run.duration_ms)}</TableCell>
							<TableCell>{formatCost(run.cost_usd)}</TableCell>
							<TableCell>
								{run.scheduled_at && !run.started_at ? (
									<span className="text-muted-foreground text-xs">
										{new Date(run.scheduled_at).toLocaleString()}
									</span>
								) : (
									<TimeAgo date={run.started_at || run.created_at} />
								)}
							</TableCell>
						</TableRow>
					))}
				</TableBody>
			</Table>
		</div>
	);
}
