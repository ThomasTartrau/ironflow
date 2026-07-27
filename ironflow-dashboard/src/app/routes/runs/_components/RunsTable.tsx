import { useNavigate } from "react-router";
import type { RunResponse } from "@/app/lib/types";
import { StatusBadge } from "@/app/components/StatusBadge";
import { TriggerBadge } from "@/app/components/TriggerBadge";
import { CreatedByBadge } from "@/app/components/CreatedByBadge";
import { TimeAgo } from "@/app/components/TimeAgo";
import { RunLabels } from "@/app/components/RunLabels";
import { formatDuration, formatCost } from "@/app/lib/format";
import { Workflow } from "lucide-react";
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
			<div className="flex flex-col items-center justify-center min-h-[200px] gap-4 border border-dashed rounded-[var(--radius)] bg-muted/20 py-12">
				<Workflow
					className="size-8 text-muted-foreground/40"
					aria-hidden="true"
				/>
				<p className="text-sm font-medium text-foreground">No runs yet</p>
				<p className="text-xs text-muted-foreground">
					Trigger a workflow to create your first run.
				</p>
			</div>
		);
	}

	const hasVersions = runs.some((r) => r.handler_version);

	return (
		<div className="rounded-[var(--radius)] border overflow-hidden">
			<Table className="table-fixed">
				<TableHeader>
					<TableRow>
						<TableHead className="w-28">Status</TableHead>
						<TableHead>Workflow</TableHead>
						{hasVersions && <TableHead className="w-20">Version</TableHead>}
						<TableHead className="w-40">Triggered by</TableHead>
						<TableHead className="w-36">Labels</TableHead>
						<TableHead className="w-24">Duration</TableHead>
						<TableHead className="w-20">Cost</TableHead>
						<TableHead className="w-36">Started / Scheduled</TableHead>
					</TableRow>
				</TableHeader>
				<TableBody>
					{runs.map((run) => (
						<TableRow
							key={run.id}
							onClick={() => handleRowClick(run.id)}
							onKeyDown={(e) => {
								if (e.key === "Enter" || e.key === " ") {
									e.preventDefault();
									handleRowClick(run.id);
								}
							}}
							tabIndex={0}
							role="link"
							aria-label={`View run for ${run.workflow_name}`}
							className="cursor-pointer hover:bg-hover-bg"
						>
							<TableCell>
								<StatusBadge status={run.status} />
							</TableCell>
							<TableCell className="font-mono font-medium truncate max-w-[220px]">
								{run.workflow_name}
							</TableCell>
							{hasVersions && (
								<TableCell className="font-mono text-xs text-muted-foreground tabular-nums">
									{run.handler_version || "-"}
								</TableCell>
							)}
							<TableCell>
								<div className="flex flex-col gap-0.5 min-w-0">
									<TriggerBadge trigger={run.trigger} />
									{run.created_by.kind !== "system" && (
										<CreatedByBadge createdBy={run.created_by} />
									)}
								</div>
							</TableCell>
							<TableCell>
								<RunLabels labels={run.labels} />
							</TableCell>
							<TableCell className="tabular-nums">
								{formatDuration(run.duration_ms)}
							</TableCell>
							<TableCell className="tabular-nums">
								{formatCost(run.cost_usd)}
							</TableCell>
							<TableCell className="tabular-nums">
								{run.scheduled_at && !run.started_at ? (
									<span className="text-muted-foreground text-xs font-mono">
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
