import { formatDuration } from "@/app/lib/format";
import type { TimelineRow } from "./types";
import { getKindMeta, ROW_HEIGHT, ROW_GAP, AXIS_HEIGHT } from "./types";

interface TimelineLabelsProps {
	rows: TimelineRow[];
	selectedStepId: string | null;
	onRowClick: (stepId: string, ownerRunId: string) => void;
}

export function TimelineLabels({
	rows,
	selectedStepId,
	onRowClick,
}: TimelineLabelsProps) {
	return (
		<div className="shrink-0 border-r border-border/20 w-[120px] md:w-[180px]">
			{rows.map((row) => {
				const meta = getKindMeta(row.step.kind);
				const Icon = meta.icon;
				const isSelected = selectedStepId === row.step.id;
				return (
					<button
						type="button"
						key={`label-${row.step.id}`}
						className={`flex items-center gap-1.5 w-full text-left transition-colors hover:bg-foreground/[0.03] ${
							isSelected ? "bg-foreground/[0.05]" : ""
						}`}
						style={{
							height: ROW_HEIGHT + ROW_GAP,
							paddingLeft: 12 + row.depth * 14,
							paddingRight: 8,
						}}
						onClick={() => onRowClick(row.step.id, row.ownerRunId)}
					>
						<Icon className={`h-3 w-3 shrink-0 ${meta.color}`} />
						<span className="text-[11px] font-medium text-foreground/70 truncate">
							{row.step.name}
						</span>
						<span className="text-[9px] text-foreground/35 ml-auto shrink-0 tabular-nums">
							{formatDuration(row.step.duration_ms)}
						</span>
					</button>
				);
			})}
			<div style={{ height: AXIS_HEIGHT }} />
		</div>
	);
}
