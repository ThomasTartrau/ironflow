import { X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "@/components/ui/select";
import { capitalize } from "@/app/lib/format";
import type { EventKind } from "@/app/lib/types";

const EVENT_KINDS: EventKind[] = [
	"run_created",
	"run_status_changed",
	"run_failed",
	"run_budget_exceeded",
	"step_completed",
	"step_failed",
	"approval_requested",
	"approval_granted",
	"approval_rejected",
	"log_line",
	"user_signed_in",
	"user_signed_up",
	"user_signed_out",
	"secrets_rotated",
	"retry_forced",
];

interface FilterValues {
	event_type: string;
	run_id: string;
	from: string;
	to: string;
}

interface AuditLogsFiltersProps {
	filters: FilterValues;
	onFilterChange: (updates: Partial<FilterValues>) => void;
	onReset: () => void;
}

export function AuditLogsFilters({
	filters,
	onFilterChange,
	onReset,
}: AuditLogsFiltersProps) {
	const activeCount = [
		filters.event_type,
		filters.run_id,
		filters.from,
		filters.to,
	].filter(Boolean).length;

	return (
		<div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-[auto_auto_auto_auto_auto] gap-3 items-end">
			<div className="grid gap-1.5">
				<label htmlFor="filter-event-type" className="text-xs font-medium">
					Event type
				</label>
				<Select
					value={filters.event_type}
					onValueChange={(v) => onFilterChange({ event_type: v || "" })}
				>
					<SelectTrigger id="filter-event-type" className="h-9">
						<SelectValue placeholder="All events" />
					</SelectTrigger>
					<SelectContent>
						<SelectItem value="">All events</SelectItem>
						{EVENT_KINDS.map((kind) => (
							<SelectItem key={kind} value={kind}>
								{capitalize(kind)}
							</SelectItem>
						))}
					</SelectContent>
				</Select>
			</div>

			<div className="grid gap-1.5">
				<label htmlFor="filter-run-id" className="text-xs font-medium">
					Run ID
				</label>
				<Input
					id="filter-run-id"
					placeholder="Full UUID..."
					value={filters.run_id}
					onChange={(e) => onFilterChange({ run_id: e.target.value })}
					className="h-9 font-mono text-xs"
				/>
			</div>

			<div className="grid gap-1.5">
				<label htmlFor="filter-from" className="text-xs font-medium">
					From
				</label>
				<Input
					id="filter-from"
					type="datetime-local"
					value={filters.from}
					onChange={(e) => onFilterChange({ from: e.target.value })}
					className="h-9 text-xs"
				/>
			</div>

			<div className="grid gap-1.5">
				<label htmlFor="filter-to" className="text-xs font-medium">
					To
				</label>
				<Input
					id="filter-to"
					type="datetime-local"
					value={filters.to}
					onChange={(e) => onFilterChange({ to: e.target.value })}
					className="h-9 text-xs"
				/>
			</div>

			{activeCount > 0 && (
				<Button
					variant="ghost"
					size="sm"
					onClick={onReset}
					className="h-9 text-muted-foreground"
				>
					<X className="h-3.5 w-3.5 mr-1" />
					Clear filters
				</Button>
			)}
		</div>
	);
}
