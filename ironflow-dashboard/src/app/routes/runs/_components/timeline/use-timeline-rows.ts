import { useState, useEffect } from "react";
import type { StepResponse, RunDetailResponse } from "@/app/lib/types";
import { api } from "@/app/lib/api";
import type { FlatStep, TimelineRow } from "./types";

function flattenSteps(
	steps: StepResponse[],
	runStartMs: number,
	ownerRunId: string,
	depth: number,
): Promise<FlatStep[]> {
	const promises = steps
		.filter((step) => step.started_at)
		.map((step) => {
			const startMs = new Date(step.started_at!).getTime() - runStartMs;
			const endMs = startMs + step.duration_ms;
			const fallback: FlatStep = { step, ownerRunId, startMs, endMs, depth };

			if (
				step.kind === "workflow" &&
				step.output &&
				typeof step.output.run_id === "string"
			) {
				const childRunId = step.output.run_id;
				return api
					.get<RunDetailResponse>(`/runs/${childRunId}`)
					.then((res) => {
						if (res.data.steps.length > 0) {
							return flattenSteps(
								res.data.steps,
								runStartMs,
								childRunId,
								depth + 1,
							);
						}
						return [fallback];
					})
					.catch(() => [fallback]);
			}

			return Promise.resolve([fallback]);
		});

	return Promise.all(promises).then((nested) => nested.flat());
}

function buildRows(flatSteps: FlatStep[], totalMs: number): TimelineRow[] {
	if (totalMs <= 0 || flatSteps.length === 0) return [];

	const sorted = [...flatSteps].sort(
		(a, b) => a.startMs - b.startMs || a.endMs - b.endMs,
	);

	return sorted.map(({ step, ownerRunId, startMs, endMs, depth }) => ({
		step,
		ownerRunId,
		startMs,
		endMs,
		depth,
		offsetPercent: (startMs / totalMs) * 100,
		widthPercent: Math.max(((endMs - startMs) / totalMs) * 100, 0.3),
	}));
}

export function useTimelineRows(
	steps: StepResponse[],
	runStartedAt: string | null,
	runId: string,
): { rows: TimelineRow[]; totalMs: number } {
	const [rows, setRows] = useState<TimelineRow[]>([]);
	const [totalMs, setTotalMs] = useState(0);

	useEffect(() => {
		if (!runStartedAt || steps.length === 0) {
			setRows([]);
			setTotalMs(0);
			return;
		}

		const runStartMs = new Date(runStartedAt).getTime();
		if (Number.isNaN(runStartMs)) {
			setRows([]);
			setTotalMs(0);
			return;
		}

		let cancelled = false;

		flattenSteps(steps, runStartMs, runId, 0)
			.then((flatSteps) => {
				if (cancelled || flatSteps.length === 0) return;
				const total = Math.max(0, ...flatSteps.map((s) => s.endMs));
				setTotalMs(total);
				setRows(buildRows(flatSteps, total));
			})
			.catch(() => {
				if (!cancelled) {
					setRows([]);
					setTotalMs(0);
				}
			});

		return () => {
			cancelled = true;
		};
	}, [steps, runStartedAt, runId]);

	return { rows, totalMs };
}
