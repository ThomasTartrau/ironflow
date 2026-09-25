import { describe, it, expect } from "vitest";
import type { StatsHistoryBucketResponse } from "@/app/lib/types";
import { STATUS_SERIES, toChartData } from "./stats-chart-data";

function bucketFixture(
	overrides: Partial<StatsHistoryBucketResponse> = {},
): StatsHistoryBucketResponse {
	return {
		time: "2026-09-21T00:00:00Z",
		completed: 0,
		warning: 0,
		failed: 0,
		cancelled: 0,
		pending: 0,
		running: 0,
		retrying: 0,
		awaiting_approval: 0,
		sleeping: 0,
		success_rate_percent: null,
		avg_duration_ms: 0,
		p95_duration_ms: 0,
		total_cost_usd: 0,
		...overrides,
	};
}

describe("toChartData", () => {
	it("computes the cumulative cost as a running sum", () => {
		const data = toChartData(
			[
				bucketFixture({ total_cost_usd: 1.5 }),
				bucketFixture({ total_cost_usd: 0 }),
				bucketFixture({ total_cost_usd: 0.25 }),
			],
			"7d",
		);
		expect(data.map((d) => d.cost)).toEqual([1.5, 0, 0.25]);
		expect(data.map((d) => d.cumulative_cost)).toEqual([1.5, 1.5, 1.75]);
	});

	it("keeps a null success rate as null, not 0", () => {
		const [datum] = toChartData(
			[bucketFixture({ running: 2, success_rate_percent: null })],
			"24h",
		);
		expect(datum.success_rate).toBeNull();
	});

	it("rounds a known success rate", () => {
		const [datum] = toChartData(
			[bucketFixture({ success_rate_percent: 66.666 })],
			"7d",
		);
		expect(datum.success_rate).toBe(67);
	});

	it("copies every status counter and sums them in total", () => {
		const [datum] = toChartData(
			[
				bucketFixture({
					completed: 1,
					warning: 2,
					failed: 3,
					cancelled: 4,
					running: 5,
					pending: 6,
					retrying: 7,
					awaiting_approval: 8,
					sleeping: 9,
				}),
			],
			"7d",
		);
		for (const s of STATUS_SERIES) {
			expect(datum[s.key]).toBeGreaterThan(0);
		}
		expect(datum.awaiting_approval).toBe(8);
		expect(datum.total).toBe(45);
	});

	it("has a series for every status", () => {
		const keys = STATUS_SERIES.map((s) => s.key).sort((a, b) =>
			a.localeCompare(b),
		);
		expect(keys).toEqual([
			"awaiting_approval",
			"cancelled",
			"completed",
			"failed",
			"pending",
			"retrying",
			"running",
			"sleeping",
			"warning",
		]);
	});

	it("labels day buckets with their UTC date", () => {
		const [datum] = toChartData(
			[bucketFixture({ time: "2026-09-21T00:00:00Z" })],
			"7d",
		);
		expect(datum.time).toContain("21");
	});
});
