import { describe, it, expect } from "vitest";
import type { RunResponse } from "@/app/lib/types";
import {
	nextPrioritySort,
	parsePriorityFilter,
	runPriority,
	sortRunsByPriority,
} from "./priority";

function run(id: string, priority?: number): RunResponse {
	const now = "2026-01-01T00:00:00Z";
	return {
		id,
		workflow_name: "deploy",
		status: "pending",
		trigger: { kind: "api" },
		error: null,
		retry_count: 0,
		max_retries: 0,
		cost_usd: 0,
		duration_ms: 0,
		created_at: now,
		updated_at: now,
		started_at: null,
		completed_at: null,
		handler_version: null,
		labels: {},
		scheduled_at: null,
		created_by: { kind: "system", id: null, label: "api" },
		priority,
	};
}

const ids = (runs: RunResponse[]) => runs.map((r) => r.id);

describe("runPriority", () => {
	it("returns the run priority", () => {
		expect(runPriority(run("a", -40))).toBe(-40);
	});

	it("treats a missing priority as 0", () => {
		expect(runPriority(run("a"))).toBe(0);
	});
});

describe("nextPrioritySort", () => {
	it("cycles unsorted, highest first, lowest first, unsorted", () => {
		expect(nextPrioritySort(null)).toBe("desc");
		expect(nextPrioritySort("desc")).toBe("asc");
		expect(nextPrioritySort("asc")).toBeNull();
	});
});

describe("sortRunsByPriority", () => {
	const runs = [
		run("low", -10),
		run("high", 50),
		run("default"),
		run("top", 100),
	];

	it("keeps the API order when unsorted", () => {
		expect(sortRunsByPriority(runs, null)).toBe(runs);
	});

	it("puts the highest priority first", () => {
		expect(ids(sortRunsByPriority(runs, "desc"))).toEqual([
			"top",
			"high",
			"default",
			"low",
		]);
	});

	it("puts the lowest priority first", () => {
		expect(ids(sortRunsByPriority(runs, "asc"))).toEqual([
			"low",
			"default",
			"high",
			"top",
		]);
	});

	it("keeps the API order between runs of the same priority", () => {
		const tied = [run("first", 5), run("second", 5), run("third", 5)];
		expect(ids(sortRunsByPriority(tied, "desc"))).toEqual([
			"first",
			"second",
			"third",
		]);
		expect(ids(sortRunsByPriority(tied, "asc"))).toEqual([
			"first",
			"second",
			"third",
		]);
	});

	it("does not mutate the input", () => {
		const before = ids(runs);
		sortRunsByPriority(runs, "desc");
		expect(ids(runs)).toEqual(before);
	});

	it("returns an empty list for no runs", () => {
		expect(sortRunsByPriority([], "desc")).toEqual([]);
	});
});

describe("parsePriorityFilter", () => {
	it("parses a whole number within the bounds", () => {
		expect(parsePriorityFilter("42")).toBe(42);
		expect(parsePriorityFilter("-7")).toBe(-7);
		expect(parsePriorityFilter(" 0 ")).toBe(0);
	});

	it("accepts both bounds", () => {
		expect(parsePriorityFilter("-100")).toBe(-100);
		expect(parsePriorityFilter("100")).toBe(100);
	});

	it("rejects a value outside the bounds", () => {
		expect(parsePriorityFilter("101")).toBeNull();
		expect(parsePriorityFilter("-101")).toBeNull();
	});

	it("rejects an empty, partial or non-integer entry", () => {
		expect(parsePriorityFilter("")).toBeNull();
		expect(parsePriorityFilter("-")).toBeNull();
		expect(parsePriorityFilter("1.5")).toBeNull();
		expect(parsePriorityFilter("1e2")).toBeNull();
		expect(parsePriorityFilter("abc")).toBeNull();
	});
});
