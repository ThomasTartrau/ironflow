import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { StepResponse } from "@/app/lib/types";
import {
	AccountBadge,
	ApprovalProgress,
	describeApprovalReason,
	StepList,
	StepTokenUsage,
	shellCommandLine,
} from "./StepList";

const RUN_ID = "019a3f2b-0000-7000-8000-0000000000ff";

type ApprovalRequirement = NonNullable<StepResponse["approval_requirement"]>;

function requirementFixture(
	overrides: Partial<ApprovalRequirement> = {},
): ApprovalRequirement {
	return {
		reason: "amount > 10k",
		required_approvers: 3,
		approver_groups: ["finance"],
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

	it("counts a gate without approvers against a single approval", () => {
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

describe("describeApprovalReason", () => {
	it("shows the reason the workflow gave for its approvers", () => {
		expect(describeApprovalReason(requirementFixture())).toBe(
			"Reason: amount > 10k",
		);
	});

	it("says so when the workflow gave no reason", () => {
		expect(describeApprovalReason(requirementFixture({ reason: null }))).toBe(
			"No reason given",
		);
	});

	it("treats a missing reason like an absent one", () => {
		expect(
			describeApprovalReason(requirementFixture({ reason: undefined })),
		).toBe("No reason given");
	});
});

describe("shellCommandLine", () => {
	it("shows a shell-mode command as written", () => {
		expect(shellCommandLine({ command: "cargo build | tee log" })).toBe(
			"cargo build | tee log",
		);
	});

	it("shows an exec-mode program followed by its arguments", () => {
		expect(shellCommandLine({ command: "git", args: ["diff", "--stat"] })).toBe(
			"git diff --stat",
		);
	});

	it("quotes an argument holding spaces, quotes or control characters", () => {
		expect(
			shellCommandLine({
				command: "printf",
				args: ["%s\n", "x'; touch /tmp/pwned; echo '", ""],
			}),
		).toBe(`printf "%s\\n" "x'; touch /tmp/pwned; echo '" ""`);
	});

	it("returns null without a command string", () => {
		expect(shellCommandLine({ args: ["a"] })).toBeNull();
		expect(shellCommandLine({ command: 42 })).toBeNull();
	});

	it("ignores args that are not a list of strings", () => {
		expect(shellCommandLine({ command: "ls", args: "-la" })).toBe("ls");
		expect(shellCommandLine({ command: "ls", args: [1, 2] })).toBe("ls");
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

describe("StepList long step names", () => {
	const longName =
		"thread api/crates/rest/companies/src/companies/edit-database-lock-acquired-before-permission-check";

	function longStep(overrides: Partial<StepResponse> = {}): StepResponse {
		return stepFixture({
			name: longName,
			kind: "gitlab",
			status: "completed",
			approval_requirement: undefined,
			approvals: [],
			approvals_required: undefined,
			...overrides,
		});
	}

	function expectFlatRow(container: HTMLElement) {
		expect(container.querySelectorAll("td td")).toHaveLength(0);
		const headers = container.querySelectorAll("th");
		const cells = container
			.querySelectorAll("tbody tr")[0]
			.querySelectorAll("td");
		expect(headers).toHaveLength(6);
		expect(cells).toHaveLength(headers.length);
	}

	it("renders the name in a truncate element without nested cells", () => {
		const errorSpy = vi.spyOn(console, "error").mockImplementation(() => {});
		const { container } = render(<StepList steps={[longStep()]} />);

		expect(errorSpy).not.toHaveBeenCalled();
		errorSpy.mockRestore();
		expectFlatRow(container);
		expect(screen.getByText(longName).classList.contains("truncate")).toBe(
			true,
		);
	});

	it("keeps the running dot beside a truncated name", () => {
		const errorSpy = vi.spyOn(console, "error").mockImplementation(() => {});
		const { container } = render(
			<StepList steps={[longStep({ status: "running" })]} />,
		);

		expect(errorSpy).not.toHaveBeenCalled();
		errorSpy.mockRestore();
		expectFlatRow(container);
		expect(container.querySelector(".animate-ping")).not.toBeNull();
		expect(screen.getByText(longName).classList.contains("truncate")).toBe(
			true,
		);
	});

	it("truncates the shortened name of a URL step", () => {
		const { container } = render(
			<StepList
				steps={[
					longStep({ name: "fetch-https://example.com/a/very/long/path" }),
				]}
			/>,
		);

		expectFlatRow(container);
		expect(
			screen.getByText("fetch: example.com").classList.contains("truncate"),
		).toBe(true);
	});
});

describe("AccountBadge", () => {
	const ACCOUNT_ID = "0192f0c1-aaaa-7000-8000-000000000001";

	it("shows the account name instead of the id", () => {
		render(
			<AccountBadge
				step={stepFixture({
					account_id: ACCOUNT_ID,
					account_name: "perso",
					account_display_name: "Compte perso",
				})}
			/>,
		);

		expect(screen.getByText("account perso")).toBeInTheDocument();
		expect(screen.queryByText(/0192f0c1/)).not.toBeInTheDocument();
	});

	it("falls back to the first 8 chars of the id without a name", () => {
		render(<AccountBadge step={stepFixture({ account_id: ACCOUNT_ID })} />);

		expect(screen.getByText("account 0192f0c1")).toBeInTheDocument();
	});

	it("renders nothing without an account id", () => {
		const { container } = render(<AccountBadge step={stepFixture()} />);

		expect(container).toBeEmptyDOMElement();
	});
});
