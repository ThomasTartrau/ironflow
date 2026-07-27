import { describe, it, expect } from "vitest";
import type { StepResponse } from "@/app/lib/types";
import { attemptLabel, listAttempts, resolveShownAttempt } from "./attempts";

function step(name: string, attempt: number): StepResponse {
	return {
		id: `${name}-${attempt}`,
		run_id: "run-1",
		name,
		kind: "shell",
		position: 0,
		status: "completed",
		attempt,
		duration_ms: 0,
		cost_usd: 0,
		created_at: "2026-07-26T10:00:00Z",
		updated_at: "2026-07-26T10:00:00Z",
		dependencies: [],
	} as unknown as StepResponse;
}

describe("listAttempts", () => {
	it("returns nothing when the run has no steps", () => {
		expect(listAttempts([])).toEqual([]);
	});

	it("lists a single attempt for a run that never retried", () => {
		expect(listAttempts([step("build", 1), step("test", 1)])).toEqual([1]);
	});

	it("deduplicates and orders attempts oldest first", () => {
		const steps = [
			step("build", 2),
			step("test", 1),
			step("build", 1),
			step("test", 3),
		];
		expect(listAttempts(steps)).toEqual([1, 2, 3]);
	});
});

describe("attemptLabel", () => {
	it("marks the latest attempt", () => {
		expect(attemptLabel(3, 3)).toBe("3 (latest)");
	});

	it("marks an earlier attempt as retried", () => {
		expect(attemptLabel(1, 3)).toBe("1 (retried)");
	});
});

describe("resolveShownAttempt", () => {
	it("defaults to the latest attempt", () => {
		expect(resolveShownAttempt(null, [1, 2, 3])).toBe(3);
	});

	it("honours an attempt that exists", () => {
		expect(resolveShownAttempt(2, [1, 2, 3])).toBe(2);
	});

	it("falls back to the latest when the requested attempt is gone", () => {
		expect(resolveShownAttempt(7, [1, 2])).toBe(2);
	});

	it("falls back to attempt 1 when the run has no steps yet", () => {
		expect(resolveShownAttempt(null, [])).toBe(1);
	});
});
