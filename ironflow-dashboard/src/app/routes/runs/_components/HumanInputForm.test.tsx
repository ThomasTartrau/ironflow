import { fireEvent, render, screen } from "@testing-library/react";
import { createMemoryRouter, RouterProvider } from "react-router";
import { describe, expect, it } from "vitest";
import type { StepResponse } from "@/app/lib/types";
import { HumanInputForm } from "./HumanInputForm";

function stepFixture(overrides: Partial<StepResponse> = {}): StepResponse {
	return {
		id: "019a3f2b-0000-7000-8000-000000000001",
		trace_id: "019a3f2b-0000-7000-8000-000000000002",
		run_id: "019a3f2b-0000-7000-8000-0000000000ff",
		name: "clarify",
		kind: "human_input",
		position: 0,
		status: "awaiting_approval",
		attempt: 1,
		duration_ms: 0,
		cost_usd: 0,
		created_at: "2026-01-01T00:00:00Z",
		updated_at: "2026-01-01T00:00:00Z",
		dependencies: [],
		approvals: [],
		input: {
			message: "Answer the clarification questions",
			schema: {
				type: "object",
				required: ["answers"],
				properties: {
					answers: { type: "array", items: { type: "string" } },
				},
			},
		},
		...overrides,
	};
}

/** Render inside a real data router: the form revalidates the route. */
async function renderForm(step: StepResponse = stepFixture()) {
	const router = createMemoryRouter([
		{ path: "/", element: <HumanInputForm step={step} /> },
	]);
	render(<RouterProvider router={router} />);
	return screen.findByLabelText("Answer (JSON)");
}

function submitButton(): HTMLElement {
	return screen.getByRole("button", { name: "Submit" });
}

describe("HumanInputForm", () => {
	it("renders the message stored on the step", async () => {
		await renderForm();

		expect(
			screen.getByText("Answer the clarification questions"),
		).toBeInTheDocument();
		expect(screen.getByText("Expected schema")).toBeInTheDocument();
	});

	it("disables Submit on invalid JSON", async () => {
		const textarea = await renderForm();

		fireEvent.change(textarea, { target: { value: "{not json" } });

		expect(submitButton()).toBeDisabled();
		expect(screen.getByText(/^Invalid JSON/)).toBeInTheDocument();
	});

	it("lists schema errors and disables Submit", async () => {
		await renderForm();

		expect(
			screen.getByText('(root): missing required property "answers"'),
		).toBeInTheDocument();
		expect(submitButton()).toBeDisabled();
	});

	it("enables Submit on a valid answer", async () => {
		const textarea = await renderForm();

		fireEvent.change(textarea, {
			target: { value: '{"answers": ["staging"]}' },
		});

		expect(submitButton()).toBeEnabled();
		expect(screen.queryByRole("listitem")).not.toBeInTheDocument();
	});

	it("keeps Reject available whatever the answer", async () => {
		await renderForm();

		expect(screen.getByRole("button", { name: "Reject" })).toBeEnabled();
	});
});
