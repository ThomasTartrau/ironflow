import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import {
	render,
	screen,
	waitFor,
	fireEvent,
	within,
} from "@testing-library/react";
import { StatsCharts } from "./StatsCharts";
import type { DashboardFilters } from "../stats-filters";

const AUTHOR_ID = "019a3f2b-0000-7000-8000-000000000001";

const filters: DashboardFilters = {
	workflow: "",
	status: "failed",
	has_steps: true,
	label: ["env:prod"],
	created_by: AUTHOR_ID,
};

let mockFetch: ReturnType<typeof vi.fn>;

beforeEach(() => {
	mockFetch = vi.fn();
	vi.stubGlobal("fetch", mockFetch);
	vi.stubGlobal(
		"ResizeObserver",
		class {
			observe() {}
			unobserve() {}
			disconnect() {}
		},
	);
	vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue({
		width: 800,
		height: 220,
		top: 0,
		left: 0,
		bottom: 220,
		right: 800,
		x: 0,
		y: 0,
		toJSON() {},
	} as DOMRect);
});

afterEach(() => {
	vi.unstubAllGlobals();
	vi.restoreAllMocks();
});

function jsonResponse(data: unknown, status = 200): Response {
	return new Response(JSON.stringify(data), {
		status,
		headers: { "Content-Type": "application/json" },
	});
}

describe("StatsCharts", () => {
	it("forwards every dashboard filter to the history request", async () => {
		mockFetch.mockResolvedValue(
			jsonResponse({
				data: { period: "24h", granularity: "1h", workflow: null, buckets: [] },
			}),
		);

		render(<StatsCharts filters={filters} period="24h" />);

		expect(
			await screen.findByText("No data for this period."),
		).toBeInTheDocument();
		const [url, opts] = mockFetch.mock.calls[0] as [string, RequestInit];
		const params = new URL(url, "http://localhost").searchParams;
		expect(params.get("period")).toBe("24h");
		expect(params.get("status")).toBe("failed");
		expect(params.get("label")).toBe("env:prod");
		expect(params.get("has_steps")).toBe("true");
		expect(params.get("created_by")).toBe(AUTHOR_ID);
		expect(opts.signal).toBeInstanceOf(AbortSignal);
	});

	it("shows an alert and clears busy state on failure", async () => {
		mockFetch.mockRejectedValue(new Error("network down"));

		const { container } = render(<StatsCharts filters={filters} period="7d" />);

		expect(await screen.findByRole("alert")).toHaveTextContent("network down");
		await waitFor(() =>
			expect(container.firstChild).toHaveAttribute("aria-busy", "false"),
		);
		expect(screen.queryByText("No data for this period.")).toBeNull();
	});

	it("keeps the Volume & Status tooltip above the legend and hides zero-value statuses", async () => {
		mockFetch.mockResolvedValue(
			jsonResponse({
				data: {
					period: "24h",
					granularity: "1h",
					workflow: null,
					buckets: [
						{
							time: "2026-09-26T10:00:00Z",
							completed: 3,
							warning: 0,
							failed: 0,
							cancelled: 0,
							running: 0,
							pending: 0,
							retrying: 0,
							awaiting_approval: 0,
							sleeping: 0,
							avg_duration_ms: 1200,
							p95_duration_ms: 1800,
							total_cost_usd: 0.05,
							success_rate_percent: 100,
						},
					],
				},
			}),
		);

		render(<StatsCharts filters={filters} period="24h" />);

		// Scope every query to the "Volume & Status" card: the "Duration" chart
		// also has a <Legend>, so an unscoped query would be ambiguous.
		const volumeCard = screen
			.getByText("Volume & Status")
			.closest("div") as HTMLElement;

		const bar = await waitFor(() => {
			const rect = volumeCard.querySelector(".recharts-bar-rectangle");
			expect(rect).not.toBeNull();
			return rect as Element;
		});

		fireEvent.mouseOver(bar);

		const tooltipWrapper = await waitFor(() => {
			const wrapper = volumeCard.querySelector(".recharts-tooltip-wrapper");
			expect(wrapper).not.toBeNull();
			expect(wrapper).toHaveTextContent("Completed");
			return wrapper as HTMLElement;
		});
		const legendWrapper = volumeCard.querySelector(
			".recharts-legend-wrapper",
		) as HTMLElement;
		expect(legendWrapper).not.toBeNull();

		const tooltipZIndex = Number(getComputedStyle(tooltipWrapper).zIndex);
		const legendZIndex = Number(getComputedStyle(legendWrapper).zIndex);
		expect(tooltipZIndex).toBeGreaterThan(legendZIndex);

		expect(within(tooltipWrapper).getByText(/Completed/)).toBeInTheDocument();
		expect(within(tooltipWrapper).queryByText(/Failed/)).toBeNull();
		expect(within(tooltipWrapper).queryByText(/Pending/)).toBeNull();
	});
});
