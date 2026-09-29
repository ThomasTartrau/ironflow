import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { createMemoryRouter, RouterProvider } from "react-router";
import { describe, expect, it, vi, beforeEach } from "vitest";
import type { StepResponse } from "@/app/lib/types";

vi.mock("../_actions/actions", () => ({
	submitStepInput: vi.fn().mockResolvedValue({}),
	rejectStepInput: vi.fn().mockResolvedValue({}),
}));

import { submitStepInput } from "../_actions/actions";
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
	beforeEach(() => {
		vi.clearAllMocks();
	});

	it("renders the message as markdown", async () => {
		await renderForm();

		expect(
			screen.getByText("Answer the clarification questions"),
		).toBeInTheDocument();
		expect(screen.getByText("Expected schema")).toBeInTheDocument();
	});

	it("falls back to the JSON editor for a schema with an array property, pre-filled with a skeleton", async () => {
		const textarea = await renderForm();

		expect((textarea as HTMLTextAreaElement).value).toBe(
			JSON.stringify({ answers: [] }, null, 2),
		);
		expect(
			screen.queryByRole("button", { name: "Form" }),
		).not.toBeInTheDocument();
		expect(
			screen.queryByRole("button", { name: "JSON" }),
		).not.toBeInTheDocument();
	});

	it("disables Submit on invalid JSON", async () => {
		const textarea = await renderForm();

		fireEvent.change(textarea, { target: { value: "{not json" } });

		expect(submitButton()).toBeDisabled();
		expect(screen.getByText(/^Invalid JSON/)).toBeInTheDocument();
	});

	it("lists schema errors only after the answer is touched", async () => {
		const textarea = await renderForm();

		expect(
			screen.queryByText('(root): missing required property "answers"'),
		).not.toBeInTheDocument();

		fireEvent.change(textarea, { target: { value: "{}" } });

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

	it('generates a single textarea field for a {reply: string} schema, enables Submit after typing, and submits {"reply": ...}', async () => {
		const step = stepFixture({
			input: {
				message: "Please reply",
				schema: {
					type: "object",
					required: ["reply"],
					properties: { reply: { type: "string" } },
				},
			},
		});
		const router = createMemoryRouter([
			{ path: "/", element: <HumanInputForm step={step} /> },
		]);
		render(<RouterProvider router={router} />);

		const field = await screen.findByLabelText("reply");
		expect(field).toBeInTheDocument();
		expect(screen.getAllByLabelText("reply")).toHaveLength(1);
		expect(submitButton()).toBeDisabled();

		fireEvent.change(field, { target: { value: "sounds good" } });
		expect(submitButton()).toBeEnabled();

		fireEvent.click(submitButton());

		await waitFor(() =>
			expect(submitStepInput).toHaveBeenCalledWith(step.run_id, step.id, {
				reply: "sounds good",
			}),
		);
	});

	it("generates radio buttons for a string enum schema", async () => {
		const step = stepFixture({
			input: {
				message: "Pick an environment",
				schema: {
					type: "object",
					required: ["environment"],
					properties: {
						environment: { type: "string", enum: ["staging", "production"] },
					},
				},
			},
		});
		const router = createMemoryRouter([
			{ path: "/", element: <HumanInputForm step={step} /> },
		]);
		render(<RouterProvider router={router} />);

		const staging = await screen.findByRole("radio", { name: "staging" });
		const production = screen.getByRole("radio", { name: "production" });
		expect(staging).not.toBeChecked();
		expect(production).not.toBeChecked();
		expect(submitButton()).toBeDisabled();

		fireEvent.click(staging);

		expect(staging).toBeChecked();
		expect(submitButton()).toBeEnabled();
	});
});
