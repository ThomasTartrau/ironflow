import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { ExecutionPlanGraph } from "./ExecutionPlanGraph";
import type {
	ExecutionPlanResponse,
	PlannedStepResponse,
} from "@/app/lib/types";

function step(
	name: string,
	overrides: Partial<PlannedStepResponse> = {},
): PlannedStepResponse {
	return {
		name,
		kind: "shell",
		workflow: "deploy",
		depth: 0,
		depends_on: [],
		...overrides,
	};
}

function plan(
	steps: PlannedStepResponse[],
	overrides: Partial<ExecutionPlanResponse> = {},
): ExecutionPlanResponse {
	return {
		workflow: "deploy",
		steps,
		max_depth: 3,
		truncated: false,
		...overrides,
	};
}

describe("ExecutionPlanGraph", () => {
	it("renders every step name", () => {
		render(<ExecutionPlanGraph plan={plan([step("build"), step("deploy")])} />);

		expect(screen.getByText("build")).toBeInTheDocument();
		expect(screen.getByText("deploy")).toBeInTheDocument();
	});

	it("groups consecutive members of a parallel wave into one box", () => {
		render(
			<ExecutionPlanGraph
				plan={plan([
					step("build"),
					step("test", { parallel_group: "parallel-1" }),
					step("lint", { parallel_group: "parallel-1" }),
				])}
			/>,
		);

		const groups = screen.getAllByTestId("parallel-group");
		expect(groups).toHaveLength(1);
		expect(groups[0]).toHaveTextContent("parallel-1");
		expect(groups[0]).toHaveTextContent("test");
		expect(groups[0]).toHaveTextContent("lint");
	});

	it("shows the condition badge of each state", () => {
		render(
			<ExecutionPlanGraph
				plan={plan([
					step("deploy-prod", {
						condition: {
							state: "evaluated",
							expression: "env == prod",
							value: true,
						},
					}),
					step("deploy-dev", {
						kind: "skip",
						condition: { state: "skipped", reason: "not prod" },
					}),
					step("notify", {
						kind: "http",
						condition: {
							state: "unevaluable",
							expression: "build succeeded",
						},
					}),
				])}
			/>,
		);

		expect(screen.getByText("when env == prod = true")).toBeInTheDocument();
		expect(screen.getByText("skipped: not prod")).toBeInTheDocument();
		expect(screen.getByText("unevaluable: build succeeded")).toBeInTheDocument();
	});

	it("shows the estimated duration when the plan has one", () => {
		render(
			<ExecutionPlanGraph
				plan={plan([step("build", { estimated_duration_ms: 5000 })], {
					estimated_duration_ms: 5000,
				})}
			/>,
		);

		expect(screen.getAllByText("~5.0s").length).toBeGreaterThan(0);
	});

	it("warns when the plan is truncated", () => {
		render(
			<ExecutionPlanGraph
				plan={plan([step("build")], {
					truncated: true,
					incomplete_reason: "step cap of 1000 reached",
				})}
			/>,
		);

		expect(screen.getByRole("status")).toHaveTextContent(
			"Plan incomplete: step cap of 1000 reached",
		);
	});

	it("nests a sub-workflow step under its own workflow label", () => {
		render(
			<ExecutionPlanGraph
				plan={plan([
					step("child", { kind: "workflow" }),
					step("child-step", { depth: 1, workflow: "child" }),
				])}
			/>,
		);

		expect(screen.getByText("child-step")).toBeInTheDocument();
		// Once as the sub-workflow node, once as the nested box label.
		expect(screen.getAllByText("child")).toHaveLength(2);
	});
});
