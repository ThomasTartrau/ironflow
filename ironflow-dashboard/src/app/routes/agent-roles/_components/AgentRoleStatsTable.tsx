import { Bot } from "lucide-react";
import type { AgentRoleStatsEntryResponse } from "@/app/lib/types";
import { formatCost, formatDuration } from "@/app/lib/format";
import {
	Table,
	TableBody,
	TableCell,
	TableHead,
	TableHeader,
	TableRow,
} from "@/components/ui/table";

interface AgentRoleStatsTableProps {
	roles: AgentRoleStatsEntryResponse[];
	selectedRole?: string;
	onSelectRole: (role: string) => void;
}

/**
 * Cost, executions and failures of every agent role over the period. Clicking
 * a role selects it, to list its steps.
 */
export function AgentRoleStatsTable({
	roles,
	selectedRole,
	onSelectRole,
}: AgentRoleStatsTableProps) {
	if (roles.length === 0) {
		return (
			<div className="flex flex-col items-center gap-3 py-16 border border-dashed rounded-[var(--radius-xl)] bg-muted/20 text-center">
				<Bot className="size-8 text-muted-foreground/40" />
				<p className="text-sm font-medium text-foreground">
					No agent role ran in this period
				</p>
				<p className="text-xs text-muted-foreground">
					Agent steps built from an AgentRole appear here.
				</p>
			</div>
		);
	}

	return (
		<div className="rounded-[var(--radius-xl)] border overflow-x-auto">
			<Table aria-label="Agent roles">
				<TableHeader>
					<TableRow>
						<TableHead>Role</TableHead>
						<TableHead className="text-right">Executions</TableHead>
						<TableHead className="text-right">Failed</TableHead>
						<TableHead className="text-right">Cost</TableHead>
						<TableHead className="text-right">Duration</TableHead>
					</TableRow>
				</TableHeader>
				<TableBody>
					{roles.map((role) => (
						<TableRow
							key={role.agent_role}
							data-state={
								role.agent_role === selectedRole ? "selected" : undefined
							}
						>
							<TableCell className="font-medium">
								<button
									type="button"
									onClick={() => onSelectRole(role.agent_role)}
									className="text-primary hover:underline cursor-pointer"
								>
									{role.agent_role}
								</button>
							</TableCell>
							<TableCell className="text-right tabular-nums">
								{role.executions}
							</TableCell>
							<TableCell className="text-right tabular-nums">
								{role.failed}
							</TableCell>
							<TableCell className="text-right tabular-nums">
								{formatCost(role.total_cost_usd)}
							</TableCell>
							<TableCell className="text-right tabular-nums">
								{formatDuration(role.total_duration_ms)}
							</TableCell>
						</TableRow>
					))}
				</TableBody>
			</Table>
		</div>
	);
}
