import { describe, it, expect } from "vitest";
import { useState } from "react";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router";
import { RunsTable } from "./RunsTable";
import type { PrioritySort } from "./priority";
import type { CreatedBy, RunResponse } from "@/app/lib/types";

function runFixture(createdBy: CreatedBy): RunResponse {
	const now = "2026-01-01T00:00:00Z";
	return {
		id: "019a3f2b-0000-7000-8000-0000000000ff",
		workflow_name: "deploy",
		status: "completed",
		trigger: { kind: "api" },
		error: null,
		retry_count: 0,
		max_retries: 0,
		cost_usd: 0,
		duration_ms: 0,
		created_at: now,
		updated_at: now,
		started_at: null,
		completed_at: null,
		handler_version: null,
		labels: {},
		scheduled_at: null,
		created_by: createdBy,
	};
}

function renderTable(runs: RunResponse[]) {
	return render(
		<MemoryRouter>
			<RunsTable runs={runs} />
		</MemoryRouter>,
	);
}

describe("RunsTable authorship", () => {
	it("labels the trigger column 'Triggered by'", () => {
		renderTable([runFixture({ kind: "system", id: null, label: "api" })]);
		expect(screen.getByText("Triggered by")).toBeInTheDocument();
	});

	it("shows the author for a user-triggered run", () => {
		renderTable([
			runFixture({
				kind: "user",
				id: "019a3f2b-0000-7000-8000-000000000001",
				label: "alice",
			}),
		]);
		expect(screen.getByText("alice")).toBeInTheDocument();
	});

	it("shows the key and its owner for an API-key-triggered run", () => {
		renderTable([
			runFixture({
				kind: "api_key",
				id: "019a3f2b-0000-7000-8000-000000000002",
				label: "ci-deploy (alice)",
			}),
		]);
		expect(screen.getByText("ci-deploy (alice)")).toBeInTheDocument();
	});

	it("does not duplicate the trigger for a system run", () => {
		// A system label repeats the trigger badge, so only the badge shows.
		renderTable([runFixture({ kind: "system", id: null, label: "api" })]);
		expect(screen.getByText("API")).toBeInTheDocument();
		expect(screen.queryByText("api")).toBeNull();
	});
});

describe("RunsTable version column", () => {
	it("shows 'latest' when handler_version is null", () => {
		const run = runFixture({ kind: "system", id: null, label: "api" });
		const runWithVersion = {
			...run,
			id: "019a3f2b-0000-7000-8000-0000000000fe",
			handler_version: "1.2.0",
		};
		renderTable([run, runWithVersion]);
		expect(screen.getByText("latest")).toBeInTheDocument();
		expect(screen.getByText("1.2.0")).toBeInTheDocument();
	});
});

describe("RunsTable duration column", () => {
	it("shows a dash for a pending run that never started", () => {
		const run = {
			...runFixture({ kind: "system", id: null, label: "api" }),
			status: "pending" as const,
			duration_ms: 0,
			started_at: null,
		};
		renderTable([run]);
		expect(screen.getByText("-")).toBeInTheDocument();
		expect(screen.queryByText("0ms")).toBeNull();
	});

	it("shows the recorded duration of a completed run", () => {
		const run = {
			...runFixture({ kind: "system", id: null, label: "api" }),
			status: "completed" as const,
			duration_ms: 2500,
			started_at: "2026-01-01T00:00:00Z",
		};
		renderTable([run]);
		expect(screen.getByText("2.5s")).toBeInTheDocument();
	});
});

describe("RunsTable priority column", () => {
	function prioritized(id: string, workflow: string, priority: number) {
		return {
			...runFixture({ kind: "system", id: null, label: "api" }),
			id,
			workflow_name: workflow,
			priority,
		};
	}

	const runs = [
		prioritized("019a3f2b-0000-7000-8000-000000000011", "nightly", -20),
		prioritized("019a3f2b-0000-7000-8000-000000000012", "hotfix", 90),
		prioritized("019a3f2b-0000-7000-8000-000000000013", "build", 0),
	];

	function SortableTable() {
		const [sort, setSort] = useState<PrioritySort | null>(null);
		return (
			<MemoryRouter>
				<RunsTable
					runs={runs}
					prioritySort={sort}
					onPrioritySortChange={setSort}
				/>
			</MemoryRouter>
		);
	}

	const workflowOrder = () =>
		screen
			.getAllByRole("link", { name: /^View run for / })
			.map((row) => row.getAttribute("aria-label"));

	it("shows the priority of each run", () => {
		renderTable(runs);
		expect(screen.getByText("Priority")).toBeInTheDocument();
		expect(screen.getByText("-20")).toBeInTheDocument();
		expect(screen.getByText("90")).toBeInTheDocument();
	});

	it("shows 0 for a run without a priority", () => {
		// The fixture leaves `priority` out, like a run serialized before the field existed.
		renderTable([runFixture({ kind: "system", id: null, label: "api" })]);
		expect(screen.getByText("0")).toBeInTheDocument();
	});

	it("is not sortable without a sort handler", () => {
		renderTable(runs);
		expect(screen.queryByRole("button", { name: /Priority/ })).toBeNull();
	});

	it("cycles the sort when the header is clicked", async () => {
		const user = userEvent.setup();
		render(<SortableTable />);
		const header = screen.getByRole("columnheader", { name: /Priority/ });
		const button = screen.getByRole("button", { name: /Priority/ });

		expect(header).not.toHaveAttribute("aria-sort");
		expect(workflowOrder()).toEqual([
			"View run for nightly",
			"View run for hotfix",
			"View run for build",
		]);

		await user.click(button);
		expect(header).toHaveAttribute("aria-sort", "descending");
		expect(workflowOrder()).toEqual([
			"View run for hotfix",
			"View run for build",
			"View run for nightly",
		]);

		await user.click(button);
		expect(header).toHaveAttribute("aria-sort", "ascending");
		expect(workflowOrder()).toEqual([
			"View run for nightly",
			"View run for build",
			"View run for hotfix",
		]);

		await user.click(button);
		expect(header).not.toHaveAttribute("aria-sort");
		expect(workflowOrder()).toEqual([
			"View run for nightly",
			"View run for hotfix",
			"View run for build",
		]);
	});
});
