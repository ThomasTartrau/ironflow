import { useCallback, useRef, useState } from "react";
import type { EventPayload } from "../lib/types";
import { useEventSource } from "./use-event-source";

export interface LogEntry {
	stepId: string;
	stepName: string;
	stream: string;
	line: string;
	at: string;
}

const MAX_LINES = 5000;

interface UseLogStreamOptions {
	runId: string;
	enabled?: boolean;
}

export function useLogStream({ runId, enabled = true }: UseLogStreamOptions) {
	const [lines, setLines] = useState<LogEntry[]>([]);
	const linesRef = useRef<LogEntry[]>([]);

	const onEvent = useCallback(
		(_kind: "log_line", data: EventPayload<"log_line">) => {
			const entry: LogEntry = {
				stepId: data.step_id,
				stepName: data.step_name,
				stream: data.stream,
				line: data.line,
				at: data.at,
			};
			linesRef.current.push(entry);
			if (linesRef.current.length > MAX_LINES) {
				linesRef.current = linesRef.current.slice(-MAX_LINES);
			}
			setLines([...linesRef.current]);
		},
		[],
	);

	useEventSource({
		runId,
		types: ["log_line"] as const,
		enabled,
		onEvent: onEvent as Parameters<typeof useEventSource>[0]["onEvent"],
	});

	const clear = useCallback(() => {
		linesRef.current = [];
		setLines([]);
	}, []);

	return { lines, clear };
}
