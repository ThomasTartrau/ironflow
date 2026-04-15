import { useRef } from "react";
import { useRevalidator } from "react-router";
import { useEventSource } from "./use-event-source";
import type { EventType } from "./use-event-source";

interface UseRevalidateOnEventOptions {
	/** Only receive events for this run. */
	runId?: string;
	/** Only receive these event types. Defaults to run lifecycle events. */
	types?: EventType[];
	/** Set to false to disable. Default: true. */
	enabled?: boolean;
}

const DEFAULT_TYPES: EventType[] = [
	"run_created",
	"run_status_changed",
	"run_failed",
];

/**
 * Revalidate the current route loader when a matching SSE event arrives.
 */
export function useRevalidateOnEvent({
	runId,
	types = DEFAULT_TYPES,
	enabled = true,
}: UseRevalidateOnEventOptions = {}) {
	const revalidator = useRevalidator();
	const revalidatorRef = useRef(revalidator);
	revalidatorRef.current = revalidator;

	useEventSource({
		runId,
		types,
		enabled,
		onEvent: () => {
			if (revalidatorRef.current.state === "idle") {
				revalidatorRef.current.revalidate();
			}
		},
	});
}
