import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { StatsCards } from "./StatsCards";
import type { StatsResponse } from "@/app/lib/types";

function statsFixture(overrides: Partial<StatsResponse> = {}): StatsResponse {
	return {
		total_runs: 10,
		success_rate_percent: 80.0,
		active_runs: 2,
		total_cost_usd: 1.5,
		completed_runs: 8,
		failed_runs: 2,
		cancelled_runs: 0,
		total_duration_ms: 5000,
		...overrides,
	};
}

describe("StatsCards", () => {
	it("displays the success rate as a percentage", () => {
		render(<StatsCards stats={statsFixture()} />);
		expect(screen.getByText("80.0%")).toBeInTheDocument();
	});

	it("displays '--' for success rate when total_runs is 0", () => {
		render(
			<StatsCards
				stats={statsFixture({
					total_runs: 0,
					success_rate_percent: 0,
					active_runs: 0,
					total_cost_usd: 0,
				})}
			/>,
		);
		expect(screen.getByText("--")).toBeInTheDocument();
		expect(screen.queryByText("0.0%")).toBeNull();
	});

	it("displays 'All time' label", () => {
		render(<StatsCards stats={statsFixture()} />);
		expect(screen.getByText("All time")).toBeInTheDocument();
	});
});
