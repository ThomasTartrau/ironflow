import type { RunDetailResponse, RunResponse } from "@/app/lib/types";

/**
 * Why a queued run may never be picked, from the workers the API saw recently.
 *
 * `null` while an eligible worker was seen, once the run left the queue, or
 * when the server does not report worker routing.
 */
export function workerRoutingWarning(
	status: RunResponse["status"],
	routing: RunDetailResponse["worker_routing"],
): string | null {
	if (status !== "pending" && status !== "retrying") return null;
	if (!routing) return null;
	if (routing.seen_workers === 0) {
		return "Aucun worker n'a été vu récemment";
	}
	if (routing.eligible_workers === 0) {
		return "Aucun worker vu récemment ne peut prendre ce run (workflow ou tags manquants)";
	}
	return null;
}
