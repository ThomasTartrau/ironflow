import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { EventPayload } from "../lib/types";
import {
	fetchRunLogHistory,
	mergeLive,
	type LogEntry,
} from "@/app/routes/runs/_components/log-history";
import { useEventSource } from "./use-event-source";

export type { LogEntry };

const MAX_LINES = 5000;

interface UseLogStreamOptions {
	runId: string;
	/** Whether the run is still producing logs (subscribe to the live stream). */
	isActive: boolean;
}

/**
 * Log lines for a run: the persisted history hydrated from
 * `GET /runs/:id/logs`, merged with the live SSE stream when the run is active.
 *
 * The SSE stream is subscribed as soon as the run is active so no line emitted
 * during hydration is lost; duplicates in the overlap window are removed by id
 * in [`mergeLive`].
 */
export function useLogStream({ runId, isActive }: UseLogStreamOptions) {
	const [history, setHistory] = useState<LogEntry[]>([]);
	const [live, setLive] = useState<LogEntry[]>([]);
	const [loading, setLoading] = useState(true);
	const [error, setError] = useState(false);
	const liveRef = useRef<LogEntry[]>([]);

	const onEvent = useCallback(
		(_kind: "log_line", data: EventPayload<"log_line">) => {
			const entry: LogEntry = {
				id: data.id ?? "",
				stepId: data.step_id,
				stepName: data.step_name,
				stream: data.stream,
				line: data.line,
				at: data.at,
			};
			liveRef.current.push(entry);
			if (liveRef.current.length > MAX_LINES) {
				liveRef.current = liveRef.current.slice(-MAX_LINES);
			}
			setLive([...liveRef.current]);
		},
		[],
	);

	useEventSource({
		runId,
		types: ["log_line"] as const,
		enabled: isActive,
		onEvent: onEvent as Parameters<typeof useEventSource>[0]["onEvent"],
	});

	useEffect(() => {
		let cancelled = false;
		setLoading(true);
		setError(false);
		fetchRunLogHistory(runId)
			.then((entries) => {
				if (!cancelled) {
					setHistory(entries);
					setLoading(false);
				}
			})
			.catch(() => {
				if (!cancelled) {
					setError(true);
					setLoading(false);
				}
			});
		return () => {
			cancelled = true;
		};
	}, [runId]);

	const lines = useMemo(() => mergeLive(history, live), [history, live]);

	const clear = useCallback(() => {
		liveRef.current = [];
		setLive([]);
		setHistory([]);
	}, []);

	return { lines, clear, loading, error };
}
