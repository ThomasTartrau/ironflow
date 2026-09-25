import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
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
});

afterEach(() => {
	vi.unstubAllGlobals();
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
});
