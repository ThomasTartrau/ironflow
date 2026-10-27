import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router";
import type { StepResponse } from "@/app/lib/types";
import { AgentRoleStepsTable } from "./AgentRoleStepsTable";

const RUN_ID = "019a3f2b-0000-7000-8000-0000000000ff";

function stepFixture(overrides: Partial<StepResponse> = {}): StepResponse {
	return {
		id: "019a3f2b-0000-7000-8000-000000000001",
		trace_id: "019a3f2b-0000-7000-8000-000000000002",
		run_id: RUN_ID,
		name: "review",
		kind: "agent",
		position: 0,
		status: "completed",
		attempt: 1,
		duration_ms: 2500,
		cost_usd: 0.42,
		created_at: "2026-01-01T00:00:00Z",
		updated_at: "2026-01-01T00:00:00Z",
		dependencies: [],
		agent_role: "Reviewer",
		...overrides,
	};
}

function renderWithRouter(ui: React.ReactElement) {
	return render(<MemoryRouter>{ui}</MemoryRouter>);
}

describe("AgentRoleStepsTable", () => {
	it("renders one row per step", () => {
		renderWithRouter(
			<AgentRoleStepsTable
				role="Reviewer"
				steps={[
					stepFixture(),
					stepFixture({
						id: "019a3f2b-0000-7000-8000-000000000003",
						name: "second-review",
						status: "failed",
					}),
				]}
			/>,
		);

		const rows = screen.getAllByRole("row");
		expect(rows).toHaveLength(3);
		expect(screen.getByText("review")).toBeInTheDocument();
		expect(screen.getByText("second-review")).toBeInTheDocument();
		expect(screen.getAllByText("$0.42")).toHaveLength(2);
		expect(screen.getAllByText("2.5s")).toHaveLength(2);
	});

	it("links each step to its run", () => {
		renderWithRouter(
			<AgentRoleStepsTable role="Reviewer" steps={[stepFixture()]} />,
		);

		const link = screen.getByRole("link", { name: RUN_ID.slice(0, 8) });
		expect(link).toHaveAttribute("href", `/runs/${RUN_ID}`);
	});

	it("shows an empty state without steps", () => {
		renderWithRouter(<AgentRoleStepsTable role="Reviewer" steps={[]} />);

		expect(
			screen.getByText("No step recorded for role Reviewer"),
		).toBeInTheDocument();
		expect(screen.queryByRole("table")).not.toBeInTheDocument();
	});
});
