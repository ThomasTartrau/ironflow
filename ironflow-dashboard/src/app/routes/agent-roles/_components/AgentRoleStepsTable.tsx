import { Link } from "react-router";
import { ListChecks } from "lucide-react";
import type { StepResponse } from "@/app/lib/types";
import { formatCost, formatDuration } from "@/app/lib/format";
import { StatusBadge } from "@/app/components/StatusBadge";
import { TimeAgo } from "@/app/components/TimeAgo";
import {
	Table,
	TableBody,
	TableCell,
	TableHead,
	TableHeader,
	TableRow,
} from "@/components/ui/table";

interface AgentRoleStepsTableProps {
	role: string;
	steps: StepResponse[];
}

/** Agent steps of one role across every run and workflow, newest first. */
export function AgentRoleStepsTable({
	role,
	steps,
}: AgentRoleStepsTableProps) {
	if (steps.length === 0) {
		return (
			<div className="flex flex-col items-center gap-3 py-12 border border-dashed rounded-[var(--radius-xl)] bg-muted/20 text-center">
				<ListChecks className="size-8 text-muted-foreground/40" />
				<p className="text-sm font-medium text-foreground">
					{`No step recorded for role ${role}`}
				</p>
			</div>
		);
	}

	return (
		<div className="rounded-[var(--radius-xl)] border overflow-x-auto">
			<Table aria-label={`Steps of role ${role}`}>
				<TableHeader>
					<TableRow>
						<TableHead>Step</TableHead>
						<TableHead>Run</TableHead>
						<TableHead>Status</TableHead>
						<TableHead className="text-right">Cost</TableHead>
						<TableHead className="text-right">Duration</TableHead>
						<TableHead>Created</TableHead>
					</TableRow>
				</TableHeader>
				<TableBody>
					{steps.map((step) => (
						<TableRow key={step.id}>
							<TableCell className="font-medium">{step.name}</TableCell>
							<TableCell className="font-mono text-xs">
								<Link
									to={`/runs/${step.run_id}`}
									className="text-primary hover:underline"
								>
									{step.run_id.slice(0, 8)}
								</Link>
							</TableCell>
							<TableCell>
								<StatusBadge status={step.status} awaitingKind={step.kind} />
							</TableCell>
							<TableCell className="text-right tabular-nums">
								{formatCost(step.cost_usd)}
							</TableCell>
							<TableCell className="text-right tabular-nums">
								{formatDuration(step.duration_ms)}
							</TableCell>
							<TableCell className="tabular-nums text-xs text-muted-foreground whitespace-nowrap">
								<TimeAgo date={step.created_at} />
							</TableCell>
						</TableRow>
					))}
				</TableBody>
			</Table>
		</div>
	);
}
