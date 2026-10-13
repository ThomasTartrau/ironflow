import { useNavigate } from "react-router";
import type { RunResponse } from "@/app/lib/types";
import { StatusBadge } from "@/app/components/StatusBadge";
import { TriggerBadge } from "@/app/components/TriggerBadge";
import { CreatedByBadge } from "@/app/components/CreatedByBadge";
import { TimeAgo } from "@/app/components/TimeAgo";
import { RunLabels } from "@/app/components/RunLabels";
import { formatRunDuration, formatCost } from "@/app/lib/format";
import { ArrowDown, ArrowUp, ArrowUpDown, Workflow } from "lucide-react";
import {
	Table,
	TableBody,
	TableCell,
	TableHead,
	TableHeader,
	TableRow,
} from "@/components/ui/table";
import { cn } from "@/lib/utils";
import {
	type PrioritySort,
	nextPrioritySort,
	runPriority,
	sortRunsByPriority,
} from "./priority";

interface RunsTableProps {
	runs: RunResponse[];
	/**
	 * Current priority sort. The API pages runs by creation date, so the sort
	 * reorders the runs of the current page only.
	 */
	prioritySort?: PrioritySort | null;
	/**
	 * Called with the next sort when the Priority header is clicked. Without
	 * it, the Priority column is shown but not sortable.
	 */
	onPrioritySortChange?: (sort: PrioritySort | null) => void;
}

const PRIORITY_ARIA_SORT = {
	desc: "descending",
	asc: "ascending",
} as const;

// Keep in sync with the column width classes below. Tailwind needs full static
// class strings, so never build these dynamically.
// 58rem fixed columns (w-40+w-24+w-40+w-48+w-24+w-20+w-36) + 12rem Workflow floor.
const TABLE_MIN_WIDTH = "min-w-[70rem]";
// Same floor plus the 5rem Version column (w-20).
const TABLE_MIN_WIDTH_WITH_VERSION = "min-w-[75rem]";

const PRIORITY_SORT_ICON = {
	desc: ArrowDown,
	asc: ArrowUp,
} as const;

export function RunsTable({
	runs,
	prioritySort = null,
	onPrioritySortChange,
}: RunsTableProps) {
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
	const sortedRuns = sortRunsByPriority(runs, prioritySort);
	const PriorityIcon = prioritySort
		? PRIORITY_SORT_ICON[prioritySort]
		: ArrowUpDown;

	return (
		<div className="rounded-[var(--radius)] border overflow-hidden">
			<Table
				className={cn(
					"table-fixed",
					hasVersions ? TABLE_MIN_WIDTH_WITH_VERSION : TABLE_MIN_WIDTH,
				)}
			>
				<TableHeader>
					<TableRow>
						<TableHead className="w-40 overflow-hidden text-ellipsis">
							Status
						</TableHead>
						<TableHead
							className="w-24 overflow-hidden text-ellipsis"
							aria-sort={
								prioritySort ? PRIORITY_ARIA_SORT[prioritySort] : undefined
							}
						>
							{onPrioritySortChange ? (
								<button
									type="button"
									onClick={() =>
										onPrioritySortChange(nextPrioritySort(prioritySort))
									}
									className="inline-flex items-center gap-1 cursor-pointer hover:text-foreground"
								>
									Priority
									<PriorityIcon className="h-3.5 w-3.5" aria-hidden="true" />
								</button>
							) : (
								"Priority"
							)}
						</TableHead>
						<TableHead className="overflow-hidden text-ellipsis">
							Workflow
						</TableHead>
						{hasVersions && (
							<TableHead className="w-20 overflow-hidden text-ellipsis">
								Version
							</TableHead>
						)}
						<TableHead className="w-40 overflow-hidden text-ellipsis">
							Triggered by
						</TableHead>
						<TableHead className="w-48 overflow-hidden text-ellipsis">
							Labels
						</TableHead>
						<TableHead className="w-24 overflow-hidden text-ellipsis">
							Duration
						</TableHead>
						<TableHead className="w-20 overflow-hidden text-ellipsis">
							Cost
						</TableHead>
						<TableHead className="w-36 overflow-hidden text-ellipsis">
							Started
						</TableHead>
					</TableRow>
				</TableHeader>
				<TableBody>
					{sortedRuns.map((run) => (
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
							<TableCell className="overflow-hidden">
								<StatusBadge status={run.status} />
							</TableCell>
							<TableCell className="font-mono tabular-nums">
								{runPriority(run)}
							</TableCell>
							<TableCell
								className="font-mono font-medium truncate"
								title={run.workflow_name}
							>
								{run.workflow_name}
							</TableCell>
							{hasVersions && (
								<TableCell className="font-mono text-xs text-muted-foreground tabular-nums">
									{run.handler_version || "latest"}
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
							<TableCell className="overflow-hidden">
								<RunLabels labels={run.labels} />
							</TableCell>
							<TableCell className="tabular-nums">
								{formatRunDuration(run)}
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
