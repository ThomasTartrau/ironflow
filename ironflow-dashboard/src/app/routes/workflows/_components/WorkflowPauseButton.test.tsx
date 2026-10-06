import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { createMemoryRouter, RouterProvider } from "react-router";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../_actions/actions", () => ({
	pauseWorkflow: vi.fn().mockResolvedValue({ paused: true }),
	resumeWorkflow: vi.fn().mockResolvedValue({ paused: false }),
}));

import { pauseWorkflow, resumeWorkflow } from "../_actions/actions";
import { WorkflowPauseButton } from "./WorkflowPauseButton";

function renderButton(pausedAt?: string) {
	const router = createMemoryRouter([
		{
			path: "/",
			element: <WorkflowPauseButton name="deploy" pausedAt={pausedAt} />,
		},
	]);
	render(<RouterProvider router={router} />);
}

describe("WorkflowPauseButton", () => {
	beforeEach(() => {
		vi.mocked(pauseWorkflow).mockClear();
		vi.mocked(resumeWorkflow).mockClear();
	});

	it("pauses a workflow that is not paused", async () => {
		renderButton();
		fireEvent.click(
			await screen.findByRole("button", { name: "Pause workflow" }),
		);

		await waitFor(() => expect(pauseWorkflow).toHaveBeenCalledWith("deploy"));
		expect(resumeWorkflow).not.toHaveBeenCalled();
	});

	it("resumes a paused workflow", async () => {
		renderButton("2026-10-06T12:00:00Z");
		fireEvent.click(
			await screen.findByRole("button", { name: "Resume workflow" }),
		);

		await waitFor(() => expect(resumeWorkflow).toHaveBeenCalledWith("deploy"));
		expect(pauseWorkflow).not.toHaveBeenCalled();
	});
});
