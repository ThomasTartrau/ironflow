import { useCallback, useEffect, useRef } from "react";
import { API_BASE } from "../lib/api";
import type { Event, EventKind, EventPayload } from "../lib/types";

export type { Event, EventKind, EventPayload };

/**
 * Static list of every event kind, derived from the generated OpenAPI union.
 * TypeScript enforces at compile time that this covers all variants.
 */
export const ALL_EVENT_KINDS = [
	"run_created",
	"run_status_changed",
	"run_failed",
	"step_completed",
	"step_failed",
	"approval_requested",
	"approval_granted",
	"approval_rejected",
	"log_line",
	"user_signed_in",
	"user_signed_up",
	"user_signed_out",
] as const satisfies readonly EventKind[];

// Exhaustiveness check: fails to compile if a new EventKind is added to the
// OpenAPI spec without being listed above.
type _ExhaustiveCheck =
	Exclude<EventKind, (typeof ALL_EVENT_KINDS)[number]> extends never
		? true
		: never;
const _exhaustive: _ExhaustiveCheck = true;
void _exhaustive;

export interface UseEventSourceOptions {
	/** Only receive events for this run. */
	runId?: string;
	/** Only receive these event types. Defaults to all kinds. */
	types?: readonly EventKind[];
	/** Called for every matching SSE event, with a fully typed payload. */
	onEvent: <K extends EventKind>(kind: K, data: EventPayload<K>) => void;
	/** Set to false to disable the connection. Default: true. */
	enabled?: boolean;
}

/**
 * Hook that opens an EventSource to `/api/v1/events` with optional filters.
 *
 * Reconnects automatically on error with a 3-second delay.
 * Closes the connection on unmount or when `enabled` becomes false.
 */
export function useEventSource({
	runId,
	types,
	onEvent,
	enabled = true,
}: UseEventSourceOptions) {
	const onEventRef = useRef(onEvent);
	onEventRef.current = onEvent;

	const typesKey = types ? [...types].sort().join(",") : "";

	const buildUrl = useCallback(() => {
		const params = new URLSearchParams();
		if (runId) params.set("run_id", runId);
		if (typesKey) params.set("types", typesKey);
		const qs = params.toString();
		return `${API_BASE}/api/v1/events${qs ? `?${qs}` : ""}`;
	}, [runId, typesKey]);

	useEffect(() => {
		if (!enabled) return;

		let es: EventSource | null = null;
		let reconnectTimer: ReturnType<typeof setTimeout> | null = null;
		let closed = false;

		function connect() {
			if (closed) return;

			const url = buildUrl();
			es = new EventSource(url, { withCredentials: true });

			const listenKinds: readonly EventKind[] = typesKey
				? (typesKey.split(",") as EventKind[])
				: ALL_EVENT_KINDS;

			for (const kind of listenKinds) {
				es.addEventListener(kind, (e: MessageEvent) => {
					const data = JSON.parse(e.data) as Event;
					onEventRef.current(kind, data as EventPayload<typeof kind>);
				});
			}

			es.onerror = () => {
				es?.close();
				if (!closed) {
					reconnectTimer = setTimeout(connect, 3000);
				}
			};
		}

		connect();

		return () => {
			closed = true;
			es?.close();
			if (reconnectTimer) clearTimeout(reconnectTimer);
		};
	}, [enabled, buildUrl, typesKey]);
}
