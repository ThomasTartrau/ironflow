import type { RunResponse } from "@/app/lib/types";

/** Direction of the priority sort on the runs table. */
export type PrioritySort = "desc" | "asc";

/** Values accepted for the `priority_sort` query param. */
export const PRIORITY_SORTS: PrioritySort[] = ["desc", "asc"];

/** Queue priority of a run. Runs created before the field existed count as 0. */
export function runPriority(run: RunResponse): number {
	return run.priority ?? 0;
}

/**
 * Cycle of the sortable header: unsorted, then highest first, then lowest
 * first, then back to unsorted.
 */
export function nextPrioritySort(
	current: PrioritySort | null,
): PrioritySort | null {
	if (current === null) return "desc";
	if (current === "desc") return "asc";
	return null;
}

/**
 * Order runs by priority without touching the input array. Runs with the same
 * priority keep the order the API returned them in.
 */
export function sortRunsByPriority(
	runs: RunResponse[],
	sort: PrioritySort | null,
): RunResponse[] {
	if (sort === null) return runs;
	const direction = sort === "desc" ? -1 : 1;
	return runs.toSorted((a, b) => direction * (runPriority(a) - runPriority(b)));
}

/** Bounds of a run priority, matching the API validation. */
export const MIN_PRIORITY = -100;
export const MAX_PRIORITY = 100;

/**
 * Parse the priority filter input. Anything that is not a whole number within
 * the bounds clears the filter rather than sending a request the API rejects.
 */
export function parsePriorityFilter(value: string): number | null {
	const trimmed = value.trim();
	if (!/^-?\d+$/.test(trimmed)) return null;
	const priority = Number(trimmed);
	if (priority < MIN_PRIORITY || priority > MAX_PRIORITY) return null;
	return priority;
}
