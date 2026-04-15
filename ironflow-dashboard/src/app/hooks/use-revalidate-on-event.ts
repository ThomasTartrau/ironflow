import { useEffect, useRef } from "react";
import { useRevalidator } from "react-router";
import type { EventKind } from "../lib/types";
import { useEventSource } from "./use-event-source";

interface UseRevalidateOnEventOptions {
	/** Only receive events for this run. */
	runId?: string;
	/** Only receive these event kinds. Defaults to run + step + approval lifecycle. */
	types?: readonly EventKind[];
	/** Set to false to disable. Default: true. */
	enabled?: boolean;
}

/**
 * Default set of event kinds that should trigger a loader revalidation on
 * run-related screens. Includes run status transitions, step outcomes and
 * approval flow, but excludes auth-only events.
 */
const DEFAULT_TYPES = [
	"run_created",
	"run_status_changed",
	"run_failed",
	"step_completed",
	"step_failed",
	"approval_requested",
	"approval_granted",
	"approval_rejected",
] as const satisfies readonly EventKind[];

/**
 * Revalidate the current route loader when a matching SSE event arrives.
 *
 * Events that arrive while a revalidation is already in flight are buffered
 * and trigger one extra reload when the revalidator returns to idle, so
 * rapid SSE bursts never leave the UI one event behind.
 */
export function useRevalidateOnEvent({
	runId,
	types = DEFAULT_TYPES,
	enabled = true,
}: UseRevalidateOnEventOptions = {}) {
	const revalidator = useRevalidator();
	const revalidatorRef = useRef(revalidator);
	revalidatorRef.current = revalidator;

	const pendingRef = useRef(false);

	useEffect(() => {
		if (revalidator.state === "idle" && pendingRef.current) {
			pendingRef.current = false;
			revalidator.revalidate();
		}
	}, [revalidator]);

	useEventSource({
		runId,
		types,
		enabled,
		onEvent: () => {
			const rev = revalidatorRef.current;
			if (rev.state === "idle") {
				rev.revalidate();
			} else {
				pendingRef.current = true;
			}
		},
	});
}
