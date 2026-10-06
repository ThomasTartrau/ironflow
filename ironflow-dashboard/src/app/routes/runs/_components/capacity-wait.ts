import type { RunResponse } from "@/app/lib/types";

/** Wake-up time shown to the user, as `HH:MM` in their locale. */
export function formatWakeTime(iso: string): string {
	return new Date(iso).toLocaleTimeString([], {
		hour: "2-digit",
		minute: "2-digit",
	});
}

/**
 * Why a sleeping run is waiting on provider capacity, and when it resumes.
 *
 * `null` unless the run sleeps because every targeted provider account is
 * rate limited. The run may resume earlier if an account is added or renewed.
 */
export function capacityWaitLabel(
	run: Pick<RunResponse, "status" | "capacity_wait_kind" | "scheduled_at">,
): string | null {
	if (run.status !== "sleeping" || !run.capacity_wait_kind) return null;
	const label = "En attente de capacité";
	if (!run.scheduled_at) return label;
	return `${label}, reprise vers ${formatWakeTime(run.scheduled_at)}`;
}
