import type { RunResponse, RunStatus } from "@/app/lib/types";

export function capitalize(str: string): string {
	return str
		.split("_")
		.map((word) => word.charAt(0).toUpperCase() + word.slice(1))
		.join(" ");
}

export function formatDuration(ms: number): string {
	if (ms < 1000) return `${ms}ms`;
	if (ms < 60000) return `${(ms / 1000).toFixed(1)}s`;
	const minutes = Math.floor(ms / 60000);
	const seconds = Math.floor((ms % 60000) / 1000);
	return `${minutes}m ${seconds}s`;
}

const TERMINAL_RUN_STATUSES: ReadonlySet<RunStatus> = new Set<RunStatus>([
	"completed",
	"failed",
	"cancelled",
	"warning",
]);

/**
 * Duration of a run for display.
 *
 * Finished runs show their recorded duration. `duration_ms` is only set once
 * a run finishes, so an in-flight run shows the time elapsed since it
 * started, and a run that never started shows `-`.
 */
export function formatRunDuration(
	run: Pick<RunResponse, "status" | "duration_ms" | "started_at">,
	now = Date.now(),
): string {
	if (TERMINAL_RUN_STATUSES.has(run.status)) {
		return formatDuration(run.duration_ms);
	}
	if (run.started_at) {
		return formatDuration(Math.max(0, now - Date.parse(run.started_at)));
	}
	return "-";
}

export function formatPercent(value: number, decimals = 1): string {
	return `${value.toFixed(decimals)}%`;
}

export function formatCost(usd: number): string {
	if (usd === 0) return "$0";
	return `$${usd.toFixed(2)}`;
}

const URL_PATTERN = /https?:\/\/([^/\s]+)/;

export function shortenStepName(name: string): string {
	const match = URL_PATTERN.exec(name);
	if (!match) return name;

	const domain = match[1];
	const urlStart = match.index;
	const prefix = name.slice(0, urlStart).replace(/-$/, "");

	if (prefix.length === 0) return domain;
	return `${prefix}: ${domain}`;
}

const BYTE_UNITS = ["B", "KB", "MB", "GB", "TB"];

/** Human-readable file size, using 1024-based units. */
export function formatBytes(bytes: number): string {
	if (!Number.isFinite(bytes) || bytes < 0) return "-";
	if (bytes < 1024) return `${bytes} B`;

	let value = bytes;
	let unit = 0;
	while (value >= 1024 && unit < BYTE_UNITS.length - 1) {
		value /= 1024;
		unit += 1;
	}
	return `${value.toFixed(value < 10 ? 1 : 0)} ${BYTE_UNITS[unit]}`;
}

/**
 * Display name for an approval assignee.
 *
 * The API sends a prefixed string (`user:{name}` or `group:{name}`); this
 * strips the prefix down to the bare name for display.
 */
export function formatAssignee(assignee: string): string {
	const separator = assignee.indexOf(":");
	return separator === -1 ? assignee : assignee.slice(separator + 1);
}

/** Countdown to an SLA deadline, in seconds. Clamped at zero. */
export function formatRemaining(seconds: number): string {
	if (!Number.isFinite(seconds) || seconds <= 0) return "expired";

	const total = Math.floor(seconds);
	const hours = Math.floor(total / 3600);
	const minutes = Math.floor((total % 3600) / 60);
	const secs = total % 60;

	if (hours > 0) return minutes === 0 ? `${hours}h` : `${hours}h ${minutes}m`;
	if (minutes > 0) return secs === 0 ? `${minutes}m` : `${minutes}m ${secs}s`;
	return `${secs}s`;
}
