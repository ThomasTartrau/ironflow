import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { StepResponse } from "@/app/lib/types";
import {
	ApprovalProgress,
	describeApprovalRule,
	StepTokenUsage,
} from "./StepList";

const RUN_ID = "019a3f2b-0000-7000-8000-0000000000ff";

type ApprovalRequirement = NonNullable<StepResponse["approval_requirement"]>;

function requirementFixture(
	overrides: Partial<ApprovalRequirement> = {},
): ApprovalRequirement {
	return {
		rule_index: 0,
		condition: "payload.amount > 10000",
		required_approvers: 3,
		approver_groups: ["finance"],
		evaluated: [],
		...overrides,
	};
}

function stepFixture(overrides: Partial<StepResponse> = {}): StepResponse {
	return {
		id: "019a3f2b-0000-7000-8000-000000000001",
		trace_id: "019a3f2b-0000-7000-8000-000000000002",
		run_id: RUN_ID,
		name: "finance-gate",
		kind: "approval",
		position: 1,
		status: "awaiting_approval",
		attempt: 1,
		duration_ms: 0,
		cost_usd: 0,
		created_at: "2026-01-01T00:00:00Z",
		updated_at: "2026-01-01T00:00:00Z",
		dependencies: [],
		approval_requirement: requirementFixture(),
		approvals: [
			{
				user_id: "019a3f2b-0000-7000-8000-00000000000a",
				approved_by: "alice",
				at: "2026-01-01T00:05:00Z",
			},
			{
				user_id: "019a3f2b-0000-7000-8000-00000000000b",
				approved_by: "bob",
				at: "2026-01-01T00:06:00Z",
			},
		],
		approvals_required: 3,
		...overrides,
	};
}

describe("ApprovalProgress", () => {
	it("counts the approvals received against the approvals required", () => {
		render(<ApprovalProgress step={stepFixture()} />);

		expect(screen.getByText("2/3 approvals")).toBeInTheDocument();
	});

	it("counts a rule-less gate against a single approval", () => {
		render(
			<ApprovalProgress
				step={stepFixture({
					approval_requirement: null,
					approvals: [],
					approvals_required: 1,
				})}
			/>,
		);

		expect(screen.getByText("0/1 approvals")).toBeInTheDocument();
	});

	it("renders nothing for a shell step", () => {
		const { container } = render(
			<ApprovalProgress
				step={stepFixture({
					kind: "shell",
					status: "completed",
					approval_requirement: null,
					approvals: [],
					approvals_required: null,
				})}
			/>,
		);

		expect(container).toBeEmptyDOMElement();
	});

	it("renders nothing when the required count is absent", () => {
		const { container } = render(
			<ApprovalProgress
				step={stepFixture({ approvals_required: undefined })}
			/>,
		);

		expect(container).toBeEmptyDOMElement();
	});
});

describe("describeApprovalRule", () => {
	it("names the matched rule and its condition", () => {
		expect(describeApprovalRule(requirementFixture())).toBe(
			"Rule #1: payload.amount > 10000",
		);
	});

	it("reports the default rule when no condition matched", () => {
		const requirement = requirementFixture({
			rule_index: null,
			condition: null,
			required_approvers: 1,
		});

		expect(describeApprovalRule(requirement)).toBe(
			"Default rule (no condition matched)",
		);
	});
});

describe("StepTokenUsage", () => {
	const agentStep = (overrides: Partial<StepResponse> = {}) =>
		stepFixture({
			name: "review",
			kind: "agent",
			status: "completed",
			approval_requirement: null,
			approvals: [],
			approvals_required: null,
			input_tokens: 100,
			output_tokens: 20,
			...overrides,
		});

	it("shows cache read and cache write tokens", () => {
		render(
			<StepTokenUsage
				step={agentStep({
					cache_read_input_tokens: 5000,
					cache_creation_input_tokens: 300,
				})}
			/>,
		);

		expect(screen.getByText("5,000 cache read")).toBeInTheDocument();
		expect(screen.getByText("300 cache write")).toBeInTheDocument();
	});

	it("hides cache tokens when they are null", () => {
		render(
			<StepTokenUsage
				step={agentStep({
					cache_read_input_tokens: null,
					cache_creation_input_tokens: null,
				})}
			/>,
		);

		expect(screen.queryByText(/cache read/)).not.toBeInTheDocument();
		expect(screen.queryByText(/cache write/)).not.toBeInTheDocument();
	});
});
