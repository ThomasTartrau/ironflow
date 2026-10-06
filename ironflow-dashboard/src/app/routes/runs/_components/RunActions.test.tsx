import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { createMemoryRouter, RouterProvider } from "react-router";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { RunResponse } from "@/app/lib/types";

vi.mock("../_actions/actions", () => ({
	cancelRun: vi.fn().mockResolvedValue({ cancelled_descendants: [] }),
	pauseRun: vi.fn().mockResolvedValue({ paused_descendants: [] }),
	resumeRun: vi.fn().mockResolvedValue({ resumed_descendants: [] }),
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

import { cancelRun, pauseRun, resumeRun } from "../_actions/actions";
import { RunActions } from "./RunActions";

const RUN_ID = "019a3f2b-0000-7000-8000-0000000000ff";

function runFixture(overrides: Partial<RunResponse> = {}): RunResponse {
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
		...overrides,
	} as RunResponse;
}

function renderActions(
	activeDescendantCount: number,
	overrides: Partial<RunResponse> = {},
) {
	const router = createMemoryRouter([
		{
			path: "/",
			element: (
				<RunActions
					run={runFixture(overrides)}
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

describe("RunActions pause and resume", () => {
	beforeEach(() => {
		vi.mocked(pauseRun).mockClear();
		vi.mocked(resumeRun).mockClear();
	});

	it("pauses a running root run", async () => {
		renderActions(0);
		fireEvent.click(await screen.findByRole("button", { name: "Pause" }));

		await waitFor(() => expect(pauseRun).toHaveBeenCalledWith(RUN_ID));
		expect(screen.queryByRole("button", { name: "Resume" })).toBeNull();
	});

	it("resumes a paused run and still offers to cancel it", async () => {
		renderActions(0, { status: "paused" });
		fireEvent.click(await screen.findByRole("button", { name: "Resume" }));

		await waitFor(() => expect(resumeRun).toHaveBeenCalledWith(RUN_ID));
		expect(screen.queryByRole("button", { name: "Pause" })).toBeNull();
		expect(screen.getByRole("button", { name: "Cancel" })).toBeTruthy();
	});

	it("offers neither on a sub-workflow run", async () => {
		renderActions(0, { trigger: { kind: "workflow" } });
		await screen.findByRole("button", { name: "Cancel" });

		expect(screen.queryByRole("button", { name: "Pause" })).toBeNull();
		expect(screen.queryByRole("button", { name: "Resume" })).toBeNull();
	});

	it("offers neither on a finished run", async () => {
		renderActions(0, { status: "completed" });
		await screen.findByRole("button", { name: "Replay" });

		expect(screen.queryByRole("button", { name: "Pause" })).toBeNull();
		expect(screen.queryByRole("button", { name: "Resume" })).toBeNull();
	});
});
