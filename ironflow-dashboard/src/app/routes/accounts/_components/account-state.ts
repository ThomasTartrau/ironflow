import type { AccountState, AccountWindowStatus } from "@/app/lib/types";

/** Observations older than this are shown as stale. */
export const STALE_AFTER_MS = 60 * 60 * 1000;

export type Tone = "success" | "warning" | "danger" | "muted";

const LABELS: Record<AccountState, string> = {
	ok: "OK",
	near_limit: "Near limit",
	limited: "Limited",
	token_invalid: "Token invalid",
	never_used: "Never used",
};

const TONES: Record<AccountState, Tone> = {
	ok: "success",
	near_limit: "warning",
	limited: "danger",
	token_invalid: "danger",
	never_used: "muted",
};

export function stateLabel(state: AccountState): string {
	return LABELS[state];
}

export function stateTone(state: AccountState): Tone {
	return TONES[state];
}

export function windowTone(status: AccountWindowStatus): Tone {
	if (status === "rejected") return "danger";
	if (status === "allowed_warning") return "warning";
	return "success";
}

/** Human countdown to `resetsAt`, e.g. `2h 05m`; `null` when unknown. */
export function formatCountdown(
	resetsAt: string | null | undefined,
	now: Date,
): string | null {
	if (!resetsAt) return null;
	const ms = new Date(resetsAt).getTime() - now.getTime();
	if (Number.isNaN(ms)) return null;
	if (ms <= 0) return "now";
	const totalMinutes = Math.floor(ms / 60_000);
	const days = Math.floor(totalMinutes / (24 * 60));
	const hours = Math.floor((totalMinutes % (24 * 60)) / 60);
	const minutes = totalMinutes % 60;
	if (days > 0) return `${days}d ${hours}h`;
	if (hours > 0) return `${hours}h ${String(minutes).padStart(2, "0")}m`;
	return `${minutes}m`;
}

export function isStale(observedAt: string, now: Date): boolean {
	return now.getTime() - new Date(observedAt).getTime() > STALE_AFTER_MS;
}

export function formatPercent(utilization: number): string {
	return `${Math.round(utilization * 100)}%`;
}
