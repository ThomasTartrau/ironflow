import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router";
import { RunsTable } from "./RunsTable";
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
