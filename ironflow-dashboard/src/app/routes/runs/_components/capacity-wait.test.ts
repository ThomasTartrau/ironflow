import { describe, it, expect } from "vitest";
import { capacityWaitLabel, formatWakeTime } from "./capacity-wait";

const WAKE_AT = "2026-10-06T14:05:00Z";

describe("capacityWaitLabel", () => {
	it("announces the capacity wait and the wake-up time", () => {
		expect(
			capacityWaitLabel({
				status: "sleeping",
				capacity_wait_kind: "claude_subscription",
				scheduled_at: WAKE_AT,
			}),
		).toBe(`En attente de capacité, reprise vers ${formatWakeTime(WAKE_AT)}`);
	});

	it("omits the time when the run has no wake-up time", () => {
		expect(
			capacityWaitLabel({
				status: "sleeping",
				capacity_wait_kind: "claude_subscription",
				scheduled_at: null,
			}),
		).toBe("En attente de capacité");
	});

	it("is null for a run sleeping on a delay or a signal", () => {
		expect(
			capacityWaitLabel({
				status: "sleeping",
				capacity_wait_kind: null,
				scheduled_at: WAKE_AT,
			}),
		).toBeNull();
		expect(
			capacityWaitLabel({ status: "sleeping", scheduled_at: WAKE_AT }),
		).toBeNull();
	});

	it("is null once the run left the sleeping state", () => {
		expect(
			capacityWaitLabel({
				status: "running",
				capacity_wait_kind: "claude_subscription",
				scheduled_at: WAKE_AT,
			}),
		).toBeNull();
	});
});

describe("formatWakeTime", () => {
	it("renders hours and minutes only", () => {
		expect(formatWakeTime(WAKE_AT)).toMatch(/^\d{1,2}:\d{2}/);
	});
});
