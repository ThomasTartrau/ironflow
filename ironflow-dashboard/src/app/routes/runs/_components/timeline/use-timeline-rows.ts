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

function isStepActive(step: StepResponse): boolean {
	return !step.completed_at && step.status === "running";
}

function applyLiveEndMs(
	flatSteps: FlatStep[],
	runStartMs: number,
	nowMs: number,
): FlatStep[] {
	const liveCutoff = nowMs - runStartMs;
	return flatSteps.map((fs) =>
		isStepActive(fs.step) && liveCutoff > fs.endMs
			? { ...fs, endMs: liveCutoff }
			: fs,
	);
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

interface UseTimelineRowsOptions {
	/** Injected clock tick. When not provided, falls back to a static snapshot. */
	nowMs?: number;
	/** If true, extend totalMs up to `nowMs - runStartMs` for live progression. */
	liveTotal?: boolean;
}

export function useTimelineRows(
	steps: StepResponse[],
	runStartedAt: string | null,
	runId: string,
	options: UseTimelineRowsOptions = {},
): { rows: TimelineRow[]; totalMs: number } {
	const [baseFlat, setBaseFlat] = useState<FlatStep[]>([]);
	const [runStartMs, setRunStartMs] = useState(0);

	useEffect(() => {
		if (!runStartedAt || steps.length === 0) {
			setBaseFlat([]);
			setRunStartMs(0);
			return;
		}

		const startMs = new Date(runStartedAt).getTime();
		if (Number.isNaN(startMs)) {
			setBaseFlat([]);
			setRunStartMs(0);
			return;
		}

		let cancelled = false;

		flattenSteps(steps, startMs, runId, 0)
			.then((flatSteps) => {
				if (cancelled) return;
				setRunStartMs(startMs);
				setBaseFlat(flatSteps);
			})
			.catch(() => {
				if (!cancelled) {
					setBaseFlat([]);
					setRunStartMs(0);
				}
			});

		return () => {
			cancelled = true;
		};
	}, [steps, runStartedAt, runId]);

	if (baseFlat.length === 0 || runStartMs === 0) {
		return { rows: [], totalMs: 0 };
	}

	const nowMs = options.nowMs ?? Date.now();
	const liveFlat = applyLiveEndMs(baseFlat, runStartMs, nowMs);
	const maxStepEnd = Math.max(0, ...liveFlat.map((s) => s.endMs));
	const totalMs = options.liveTotal
		? Math.max(maxStepEnd, nowMs - runStartMs)
		: maxStepEnd;

	return { rows: buildRows(liveFlat, totalMs), totalMs };
}
