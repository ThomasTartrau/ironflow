import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { createMemoryRouter, RouterProvider } from "react-router";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { RunResponse } from "@/app/lib/types";

vi.mock("../_actions/actions", () => ({
	cancelRun: vi.fn().mockResolvedValue({ cancelled_descendants: [] }),
	approveRun: vi.fn(),
	rejectRun: vi.fn(),
	replayRun: vi.fn(),
	retryRun: vi.fn(),
}));

vi.mock("@/app/store", () => ({
	useAppSelector: (selector: (state: { auth: unknown }) => unknown) =>
		selector({
			auth: {
				status: "authenticated",
				user: { user_id: "admin-id", is_admin: true },
			},
		}),
}));

import { cancelRun } from "../_actions/actions";
import { RunActions } from "./RunActions";

const RUN_ID = "019a3f2b-0000-7000-8000-0000000000ff";

function runFixture(): RunResponse {
	return {
		id: RUN_ID,
		workflow_name: "deploy",
		status: "running",
		trigger: { kind: "manual" },
		retry_count: 0,
		max_retries: 0,
		cost_usd: 0,
		duration_ms: 0,
		created_at: "2026-01-01T00:00:00Z",
		updated_at: "2026-01-01T00:00:00Z",
		created_by: { kind: "system", label: "cron" },
	} as RunResponse;
}

function renderActions(activeDescendantCount: number) {
	const router = createMemoryRouter([
		{
			path: "/",
			element: (
				<RunActions
					run={runFixture()}
					awaitingInput={false}
					activeDescendantCount={activeDescendantCount}
				/>
			),
		},
	]);
	render(<RouterProvider router={router} />);
}

describe("RunActions cancel confirmation", () => {
	beforeEach(() => {
		vi.mocked(cancelRun).mockClear();
	});

	it("tells how many active sub-runs the cancellation reaches", async () => {
		renderActions(2);
		fireEvent.click(await screen.findByRole("button", { name: "Cancel" }));

		expect(
			await screen.findByText("2 active sub-runs will be cancelled with it."),
		).toBeTruthy();
		expect(cancelRun).not.toHaveBeenCalled();

		fireEvent.click(screen.getByRole("button", { name: "Cancel run" }));
		await waitFor(() => expect(cancelRun).toHaveBeenCalledWith(RUN_ID));
	});

	it("uses the singular for a single sub-run", async () => {
		renderActions(1);
		fireEvent.click(await screen.findByRole("button", { name: "Cancel" }));

		expect(
			await screen.findByText("1 active sub-run will be cancelled with it."),
		).toBeTruthy();
	});

	it("says no sub-run is affected when none is active", async () => {
		renderActions(0);
		fireEvent.click(await screen.findByRole("button", { name: "Cancel" }));

		expect(await screen.findByText("No sub-run is active.")).toBeTruthy();
	});

	it("keeps the run when the confirmation is dismissed", async () => {
		renderActions(3);
		fireEvent.click(await screen.findByRole("button", { name: "Cancel" }));
		fireEvent.click(
			await screen.findByRole("button", { name: "Keep running" }),
		);

		await waitFor(() =>
			expect(screen.queryByText(/will be cancelled with it/)).toBeNull(),
		);
		expect(cancelRun).not.toHaveBeenCalled();
	});
});
