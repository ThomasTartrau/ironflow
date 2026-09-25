import type { StatsHistoryBucketResponse } from "@/app/lib/types";

export const PERIODS = ["24h", "7d", "30d", "90d"] as const;
export type Period = (typeof PERIODS)[number];

/** Per-status run counters of a history bucket. */
export type StatusKey =
	| "completed"
	| "warning"
	| "failed"
	| "cancelled"
	| "running"
	| "pending"
	| "retrying"
	| "awaiting_approval"
	| "sleeping";

export interface StatusSeries {
	key: StatusKey;
	label: string;
	color: string;
}

/** Stacking order and colors of the "Volume & Status" bars. */
export const STATUS_SERIES: readonly StatusSeries[] = [
	{
		key: "completed",
		label: "Completed",
		color: "var(--status-completed-fg)",
	},
	{ key: "warning", label: "Warning", color: "var(--status-warning-fg)" },
	{ key: "failed", label: "Failed", color: "var(--status-failed-fg)" },
	{
		key: "cancelled",
		label: "Cancelled",
		color: "var(--status-cancelled-fg)",
	},
	{ key: "running", label: "Running", color: "var(--status-running-fg)" },
	{ key: "pending", label: "Pending", color: "var(--status-pending-fg)" },
	{ key: "retrying", label: "Retrying", color: "var(--status-retrying-fg)" },
	{
		key: "awaiting_approval",
		label: "Awaiting approval",
		color: "var(--status-awaiting-fg)",
	},
	{ key: "sleeping", label: "Sleeping", color: "var(--status-sleeping-fg)" },
];

export type ChartDatum = Record<StatusKey, number> & {
	time: string;
	/** Runs created in the bucket, all statuses included. */
	total: number;
	avg_duration_ms: number;
	p95_duration_ms: number;
	cost: number;
	cumulative_cost: number;
	/** Rounded percentage, `null` when the bucket has no finished run. */
	success_rate: number | null;
};

/**
 * Label of a bucket on the X axis.
 *
 * Day and week buckets start at 00:00 UTC, so their date is rendered in UTC:
 * a local rendering would show the previous day west of Greenwich.
 */
export function formatTime(time: string, period: Period): string {
	const d = new Date(time);
	if (period === "24h") {
		return d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
	}
	return d.toLocaleDateString([], {
		month: "short",
		day: "numeric",
		timeZone: "UTC",
	});
}

export function toChartData(
	buckets: StatsHistoryBucketResponse[],
	period: Period,
): ChartDatum[] {
	let cumulativeCost = 0;
	return buckets.map((b) => {
		const cost = Number(b.total_cost_usd);
		cumulativeCost += cost;
		const counts: Record<StatusKey, number> = {
			completed: b.completed,
			warning: b.warning,
			failed: b.failed,
			cancelled: b.cancelled,
			running: b.running,
			pending: b.pending,
			retrying: b.retrying,
			awaiting_approval: b.awaiting_approval,
			sleeping: b.sleeping,
		};
		return {
			...counts,
			time: formatTime(b.time, period),
			total: STATUS_SERIES.reduce((sum, s) => sum + counts[s.key], 0),
			avg_duration_ms: b.avg_duration_ms,
			p95_duration_ms: b.p95_duration_ms,
			cost,
			cumulative_cost: cumulativeCost,
			success_rate:
				b.success_rate_percent == null
					? null
					: Math.round(b.success_rate_percent),
		};
	});
}
