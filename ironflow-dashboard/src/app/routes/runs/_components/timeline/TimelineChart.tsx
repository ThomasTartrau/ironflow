import {
	Tooltip,
	TooltipTrigger,
	TooltipContent,
} from "@/components/ui/tooltip";
import type { TimelineRow } from "./types";
import type { TimeMarker } from "./compute-time-markers";
import {
	getKindMeta,
	statusBarModifier,
	ROW_HEIGHT,
	ROW_GAP,
	AXIS_HEIGHT,
	MIN_BAR_PX,
} from "./types";
import { StepTooltipContent } from "./StepTooltipContent";

interface TimelineChartProps {
	rows: TimelineRow[];
	markers: TimeMarker[];
	totalMs: number;
	selectedStepId: string | null;
	onRowClick: (stepId: string, ownerRunId: string) => void;
}

export function TimelineChart({
	rows,
	markers,
	totalMs,
	selectedStepId,
	onRowClick,
}: TimelineChartProps) {
	const contentHeight = rows.length * (ROW_HEIGHT + ROW_GAP);

	return (
		<div className="flex-1 min-w-0">
			<div
				className="relative overflow-hidden"
				style={{ height: contentHeight }}
			>
				{rows.map((row, index) => (
					<div
						key={`bg-${row.step.id}`}
						className={index % 2 === 0 ? "" : "bg-foreground/[0.015]"}
						style={{
							position: "absolute",
							left: 0,
							right: 0,
							top: index * (ROW_HEIGHT + ROW_GAP),
							height: ROW_HEIGHT + ROW_GAP,
						}}
					/>
				))}

				{markers.map((marker) => {
					const leftPercent = totalMs > 0 ? (marker.ms / totalMs) * 100 : 0;
					return (
						<div
							key={`grid-${marker.ms}`}
							className="absolute top-0 bottom-0 w-px bg-foreground/[0.06]"
							style={{ left: `${leftPercent}%` }}
						/>
					);
				})}

				{rows.map((row, index) => {
					const meta = getKindMeta(row.step.kind);
					const isSelected = selectedStepId === row.step.id;
					const statusMod = statusBarModifier[row.step.status] ?? "";
					const top = index * (ROW_HEIGHT + ROW_GAP) + 3;
					const barHeight = ROW_HEIGHT - 4;

					return (
						<Tooltip key={row.step.id}>
							<TooltipTrigger
								className={`absolute rounded-sm border cursor-pointer transition-all ${meta.barBg} ${meta.barBorder} ${statusMod} ${
									isSelected
										? "ring-2 ring-primary/50 shadow-sm z-10"
										: "hover:brightness-110 hover:shadow-sm"
								}`}
								style={{
									left: `${row.offsetPercent}%`,
									width: `${row.widthPercent}%`,
									minWidth: MIN_BAR_PX,
									top,
									height: barHeight,
								}}
								onClick={() => onRowClick(row.step.id, row.ownerRunId)}
							/>
							<TooltipContent side="top" sideOffset={6}>
								<StepTooltipContent row={row} />
							</TooltipContent>
						</Tooltip>
					);
				})}
			</div>

			<div
				className="relative border-t border-foreground/10"
				style={{ height: AXIS_HEIGHT }}
			>
				{markers.map((marker, index) => {
					const leftPercent = totalMs > 0 ? (marker.ms / totalMs) * 100 : 0;
					const isFirst = index === 0;
					const isLast = index === markers.length - 1;
					return (
						<div
							key={`tick-${marker.ms}`}
							className="absolute top-0"
							style={{ left: `${leftPercent}%` }}
						>
							<div className="w-px h-1.5 bg-foreground/30" />
							<span
								className={`text-[10px] text-foreground/40 font-medium whitespace-nowrap inline-block mt-0.5 ${
									isFirst
										? ""
										: isLast
											? "-translate-x-full"
											: "-translate-x-1/2"
								}`}
							>
								{marker.label}
							</span>
						</div>
					);
				})}
			</div>
		</div>
	);
}
