import { describe, it, expect } from "vitest";
import {
	countActiveFilters,
	toFilterParams,
	type DashboardFilters,
} from "./stats-filters";

function filtersFixture(
	overrides: Partial<DashboardFilters> = {},
): DashboardFilters {
	return {
		workflow: "",
		status: "",
		has_steps: false,
		label: [],
		created_by: "",
		...overrides,
	};
}

describe("toFilterParams", () => {
	it("returns no param when no filter is set", () => {
		expect(toFilterParams(filtersFixture()).toString()).toBe("");
	});

	it("maps the workflow filter", () => {
		const params = toFilterParams(filtersFixture({ workflow: "deploy" }));
		expect(params.get("workflow")).toBe("deploy");
	});

	it("maps the status filter", () => {
		const params = toFilterParams(filtersFixture({ status: "failed" }));
		expect(params.get("status")).toBe("failed");
	});

	it("sends has_steps only when enabled", () => {
		expect(
			toFilterParams(filtersFixture({ has_steps: true })).get("has_steps"),
		).toBe("true");
		expect(
			toFilterParams(filtersFixture({ has_steps: false })).has("has_steps"),
		).toBe(false);
	});

	it("joins labels with a comma", () => {
		const params = toFilterParams(
			filtersFixture({ label: ["env:prod", "team:core"] }),
		);
		expect(params.get("label")).toBe("env:prod,team:core");
	});

	it("maps the author filter", () => {
		const params = toFilterParams(
			filtersFixture({ created_by: "019a3f2b-0000-7000-8000-000000000001" }),
		);
		expect(params.get("created_by")).toBe(
			"019a3f2b-0000-7000-8000-000000000001",
		);
	});
});

describe("countActiveFilters", () => {
	it("is zero with default filters", () => {
		expect(countActiveFilters(filtersFixture({ has_steps: true }))).toBe(0);
	});

	it("counts a disabled has_steps as active", () => {
		expect(countActiveFilters(filtersFixture({ has_steps: false }))).toBe(1);
	});

	it("counts every set filter, including the author", () => {
		expect(
			countActiveFilters({
				workflow: "deploy",
				status: "failed",
				has_steps: true,
				label: ["env:prod", "team:core"],
				created_by: "019a3f2b-0000-7000-8000-000000000001",
			}),
		).toBe(4);
	});
});
