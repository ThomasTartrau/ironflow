import type { StepResponse } from "@/app/lib/types";

/**
 * Every attempt that produced at least one step, oldest first.
 *
 * A run replayed after a transient failure keeps the steps of each attempt, so
 * a failed attempt stays inspectable.
 */
export function listAttempts(steps: StepResponse[]): number[] {
	const seen = new Set(steps.map((s) => s.attempt));
	return [...seen].sort((a, b) => a - b);
}

/** Label shown for an attempt in the selector. */
export function attemptLabel(attempt: number, latest: number): string {
	return attempt === latest ? `${attempt} (latest)` : `${attempt} (retried)`;
}

/**
 * The attempt the step views should show.
 *
 * Falls back to the latest attempt when the URL asks for one that does not
 * exist, rather than rendering an empty timeline.
 */
export function resolveShownAttempt(
	requested: number | null,
	attempts: number[],
): number {
	if (requested !== null && attempts.includes(requested)) return requested;
	return attempts.at(-1) ?? 1;
}
