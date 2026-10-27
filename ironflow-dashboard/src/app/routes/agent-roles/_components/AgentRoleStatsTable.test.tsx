import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import type { AgentRoleStatsEntryResponse } from "@/app/lib/types";
import { AgentRoleStatsTable } from "./AgentRoleStatsTable";

const ROLES: AgentRoleStatsEntryResponse[] = [
	{
		agent_role: "Reviewer",
		executions: 12,
		failed: 2,
		total_cost_usd: 3.5,
		total_duration_ms: 90_000,
	},
	{
		agent_role: "Writer",
		executions: 1,
		failed: 0,
		total_cost_usd: 0.25,
		total_duration_ms: 1500,
	},
];

describe("AgentRoleStatsTable", () => {
	it("renders one row per role", () => {
		render(<AgentRoleStatsTable roles={ROLES} onSelectRole={() => {}} />);

		const rows = screen.getAllByRole("row");
		expect(rows).toHaveLength(3);
		expect(screen.getByText("Reviewer")).toBeInTheDocument();
		expect(screen.getByText("Writer")).toBeInTheDocument();
		expect(screen.getByText("12")).toBeInTheDocument();
		expect(screen.getByText("$3.50")).toBeInTheDocument();
		expect(screen.getByText("1m 30s")).toBeInTheDocument();
	});

	it("selects a role on click", () => {
		const onSelectRole = vi.fn();
		render(<AgentRoleStatsTable roles={ROLES} onSelectRole={onSelectRole} />);

		fireEvent.click(screen.getByRole("button", { name: "Writer" }));

		expect(onSelectRole).toHaveBeenCalledWith("Writer");
	});

	it("marks the selected role", () => {
		render(
			<AgentRoleStatsTable
				roles={ROLES}
				selectedRole="Reviewer"
				onSelectRole={() => {}}
			/>,
		);

		const row = screen.getByText("Reviewer").closest("tr");
		expect(row).toHaveAttribute("data-state", "selected");
	});

	it("shows an empty state without roles", () => {
		render(<AgentRoleStatsTable roles={[]} onSelectRole={() => {}} />);

		expect(
			screen.getByText("No agent role ran in this period"),
		).toBeInTheDocument();
		expect(screen.queryByRole("table")).not.toBeInTheDocument();
	});
});
