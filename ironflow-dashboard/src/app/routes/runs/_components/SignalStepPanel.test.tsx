import { fireEvent, render, screen } from "@testing-library/react";
import { createMemoryRouter, RouterProvider } from "react-router";
import { describe, expect, it, vi, beforeEach } from "vitest";
import type { StepResponse } from "@/app/lib/types";

vi.mock("../_actions/actions", () => ({
	sendSignal: vi.fn().mockResolvedValue({}),
}));

const authState = vi.hoisted(() => ({
	current: {
		status: "authenticated",
		user: { user_id: "admin-id", is_admin: true },
	} as unknown,
}));

vi.mock("@/app/store", () => ({
	useAppSelector: (selector: (state: { auth: unknown }) => unknown) =>
		selector({ auth: authState.current }),
}));

import { sendSignal } from "../_actions/actions";
import { SignalStepPanel } from "./SignalStepPanel";

function stepFixture(overrides: Partial<StepResponse> = {}): StepResponse {
	return {
		id: "019a3f2b-0000-7000-8000-000000000001",
		trace_id: "019a3f2b-0000-7000-8000-000000000002",
		run_id: "019a3f2b-0000-7000-8000-0000000000ff",
		name: "wait-ci",
		kind: "signal",
		position: 0,
		status: "running",
		attempt: 1,
		duration_ms: 0,
		cost_usd: 0,
		created_at: "2026-01-01T00:00:00Z",
		updated_at: "2026-01-01T00:00:00Z",
		dependencies: [],
		approvals: [],
		input: {
			name: "ci.pipeline_finished",
			key: "4f2a9c1",
			waiting_since: "2026-01-01T00:00:00Z",
			deadline_at: "2026-01-01T01:00:00Z",
			schema: {
				type: "object",
				required: ["status"],
				properties: { status: { type: "string" } },
			},
		},
		...overrides,
	};
}

function renderPanel(step: StepResponse = stepFixture()) {
	const router = createMemoryRouter([
		{ path: "/", element: <SignalStepPanel step={step} /> },
	]);
	render(<RouterProvider router={router} />);
}

describe("SignalStepPanel", () => {
	beforeEach(() => {
		vi.mocked(sendSignal).mockClear();
		authState.current = {
			status: "authenticated",
			user: { user_id: "admin-id", is_admin: true },
		};
	});

	it("renders the signal name, key and deadline", async () => {
		renderPanel();
		expect(await screen.findByText("ci.pipeline_finished")).toBeTruthy();
		expect(screen.getByText("4f2a9c1")).toBeTruthy();
		expect(screen.getByText("2026-01-01T01:00:00Z")).toBeTruthy();
	});

	it("prefills the required properties and hides errors until edited", async () => {
		renderPanel();
		const payload = (await screen.findByLabelText(
			"Signal payload",
		)) as HTMLTextAreaElement;
		expect(JSON.parse(payload.value)).toEqual({ status: "" });
		expect(screen.queryByText(/missing required property/)).toBeNull();

		fireEvent.change(payload, { target: { value: "{}" } });
		expect(
			screen.getByText('(root): missing required property "status"'),
		).toBeTruthy();
	});

	it("disables delivery while the payload does not match the schema", async () => {
		renderPanel();
		const payload = await screen.findByLabelText("Signal payload");
		const button = screen.getByRole("button", { name: "Deliver signal" });

		fireEvent.change(payload, { target: { value: '{"status": 42}' } });
		expect((button as HTMLButtonElement).disabled).toBe(true);

		fireEvent.change(payload, { target: { value: "not json" } });
		expect((button as HTMLButtonElement).disabled).toBe(true);

		fireEvent.change(payload, { target: { value: '{"status": "success"}' } });
		expect((button as HTMLButtonElement).disabled).toBe(false);

		fireEvent.click(button);
		expect(sendSignal).toHaveBeenCalledWith("ci.pipeline_finished", "4f2a9c1", {
			status: "success",
		});
	});

	it("hides the delivery form from a non-admin", async () => {
		authState.current = {
			status: "authenticated",
			user: { user_id: "member-id", is_admin: false },
		};
		renderPanel();
		expect(await screen.findByText("ci.pipeline_finished")).toBeTruthy();
		expect(screen.queryByLabelText("Signal payload")).toBeNull();
	});

	it("hides the delivery form once the step is resolved", async () => {
		renderPanel(stepFixture({ status: "completed" }));
		expect(await screen.findByText("ci.pipeline_finished")).toBeTruthy();
		expect(screen.queryByRole("button", { name: "Deliver signal" })).toBeNull();
	});
});
