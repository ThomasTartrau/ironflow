import { useState, useCallback } from "react";
import type { StepResponse } from "@/app/lib/types";
import { TooltipProvider } from "@/components/ui/tooltip";
import { useTimelineRows } from "./timeline/use-timeline-rows";
import { computeTimeMarkers } from "./timeline/compute-time-markers";
import { focusStep } from "./timeline/focus-step";
import { TimelineLabels } from "./timeline/TimelineLabels";
import { TimelineChart } from "./timeline/TimelineChart";

interface StepTimelineProps {
	steps: StepResponse[];
	runStartedAt: string | null;
	runId: string;
	nowMs?: number;
	isRunActive?: boolean;
}

export function StepTimeline({
	steps,
	runStartedAt,
	runId,
	nowMs,
	isRunActive = false,
}: StepTimelineProps) {
	const { rows, totalMs } = useTimelineRows(steps, runStartedAt, runId, {
		nowMs,
		liveTotal: isRunActive,
	});
	const [selectedStepId, setSelectedStepId] = useState<string | null>(null);

	const markers = computeTimeMarkers(totalMs);

	const handleRowClick = useCallback(
		(stepId: string, ownerRunId: string) => {
			setSelectedStepId(stepId);
			focusStep(stepId, ownerRunId, runId);
		},
		[runId],
	);

	if (rows.length === 0) return null;

	return (
		<TooltipProvider delay={200}>
			<div className="rounded-2xl border border-border/40 bg-background/60 backdrop-blur pb-2">
				<div className="flex">
					<TimelineLabels
						rows={rows}
						selectedStepId={selectedStepId}
						onRowClick={handleRowClick}
					/>
					<TimelineChart
						rows={rows}
						markers={markers}
						totalMs={totalMs}
						selectedStepId={selectedStepId}
						onRowClick={handleRowClick}
					/>
				</div>
			</div>
		</TooltipProvider>
	);
}
