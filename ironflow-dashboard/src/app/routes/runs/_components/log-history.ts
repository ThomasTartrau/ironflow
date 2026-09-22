import { api } from "@/app/lib/api";
import type { components } from "@/app/lib/types.generated";

type ApiLogEntry = components["schemas"]["LogEntry"];
type LogCursorMeta = components["schemas"]["LogCursorMeta"];

/**
 * A single log line as rendered by the panel. Shared shape for both the
 * persisted history (`GET /runs/:id/logs`) and the live SSE stream so the two
 * can be merged and de-duplicated by `id`.
 */
export interface LogEntry {
	/** Persisted entry id (UUID v7). Empty/nil for legacy live events. */
	id: string;
	stepId: string;
	stepName: string;
	stream: string;
	line: string;
	at: string;
}

/** Optional server-side filters mirrored from the history endpoint. */
export interface LogHistoryFilters {
	stepId?: string;
	stream?: string;
}

/** Server-side maximum accepted by the logs endpoint. */
const HISTORY_PAGE_LIMIT = 1000;

/** The nil UUID: what a live event carries when its producer predates `id`. */
const NIL_UUID = "00000000-0000-0000-0000-000000000000";

/** Map an API log entry to the panel's [`LogEntry`] shape. */
export function mapApiLogEntry(entry: ApiLogEntry): LogEntry {
	return {
		id: entry.id,
		stepId: entry.step_id,
		stepName: entry.step_name,
		stream: entry.stream,
		line: entry.line,
		at: entry.created_at,
	};
}

function buildLogsPath(
	runId: string,
	filters: LogHistoryFilters,
	cursor?: string,
): string {
	const params = new URLSearchParams();
	params.set("limit", String(HISTORY_PAGE_LIMIT));
	if (cursor) params.set("cursor", cursor);
	if (filters.stepId) params.set("step_id", filters.stepId);
	if (filters.stream) params.set("stream", filters.stream);
	return `/runs/${runId}/logs?${params.toString()}`;
}

/**
 * Fetch the full persisted log history for a run, following the cursor until
 * `next_cursor` is null. Pages are concatenated in id-ascending order.
 */
export async function fetchRunLogHistory(
	runId: string,
	filters: LogHistoryFilters = {},
): Promise<LogEntry[]> {
	const all: LogEntry[] = [];
	let cursor: string | undefined;

	do {
		const res = await api.get<ApiLogEntry[]>(
			buildLogsPath(runId, filters, cursor),
		);
		for (const entry of res.data) {
			all.push(mapApiLogEntry(entry));
		}
		const meta = res.meta as unknown as Partial<LogCursorMeta> | undefined;
		cursor = meta?.next_cursor ?? undefined;
	} while (cursor);

	return all;
}

function hasStableId(entry: LogEntry): boolean {
	return entry.id !== "" && entry.id !== NIL_UUID;
}

/**
 * Merge live SSE lines onto the persisted history, dropping any live line whose
 * `id` is already present in the history (the overlap window on an active run).
 *
 * Live lines without a stable id (legacy producers) are always kept, since they
 * cannot be matched against the history.
 */
export function mergeLive(history: LogEntry[], live: LogEntry[]): LogEntry[] {
	const seen = new Set(history.filter(hasStableId).map((entry) => entry.id));
	const extra = live.filter(
		(entry) => !hasStableId(entry) || !seen.has(entry.id),
	);
	return [...history, ...extra];
}
