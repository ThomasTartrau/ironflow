import { describe, it, expect } from "vitest";
import {
	formatCountdown,
	formatPercent,
	isStale,
	kindDescription,
	kindLabel,
	percentToThreshold,
	planLabel,
	stateLabel,
	stateTone,
	thresholdToPercent,
	windowTone,
} from "./account-state";

const now = new Date("2026-09-30T12:00:00Z");

describe("account state helpers", () => {
	it("labels every state", () => {
		expect(stateLabel("ok")).toBe("OK");
		expect(stateLabel("near_limit")).toBe("Near limit");
		expect(stateLabel("limited")).toBe("Limited");
		expect(stateLabel("token_invalid")).toBe("Token invalid");
		expect(stateLabel("never_used")).toBe("Never used");
	});

	it("gives a tone to states and windows", () => {
		expect(stateTone("ok")).toBe("success");
		expect(stateTone("limited")).toBe("danger");
		expect(stateTone("never_used")).toBe("muted");
		expect(windowTone("rejected")).toBe("danger");
		expect(windowTone("allowed_warning")).toBe("warning");
		expect(windowTone("allowed")).toBe("success");
	});

	it("formats countdowns", () => {
		expect(formatCountdown(null, now)).toBeNull();
		expect(formatCountdown("2026-09-30T11:00:00Z", now)).toBe("now");
		expect(formatCountdown("2026-09-30T12:45:00Z", now)).toBe("45m");
		expect(formatCountdown("2026-09-30T14:05:00Z", now)).toBe("2h 05m");
		expect(formatCountdown("2026-10-03T15:00:00Z", now)).toBe("3d 3h");
	});

	it("detects stale observations", () => {
		expect(isStale("2026-09-30T11:30:00Z", now)).toBe(false);
		expect(isStale("2026-09-30T10:00:00Z", now)).toBe(true);
	});

	it("formats percentages", () => {
		expect(formatPercent(0.424)).toBe("42%");
	});

	it("names a known kind and falls back to the identifier", () => {
		expect(kindLabel("claude_subscription")).toBe("Claude subscription");
		expect(kindLabel("other_kind")).toBe("other_kind");
		expect(kindDescription("claude_subscription")).toContain("Pro or Max");
		expect(kindDescription("other_kind")).toBeNull();
	});

	it("formats the plan", () => {
		expect(planLabel("max")).toBe("Max plan");
		expect(planLabel("")).toBe(" plan");
	});

	it("converts the alert threshold to and from a percentage", () => {
		expect(thresholdToPercent(0.8)).toBe("80");
		expect(thresholdToPercent(0.05)).toBe("5");
		expect(thresholdToPercent(1)).toBe("100");
		expect(percentToThreshold("80")).toBe(0.8);
		expect(percentToThreshold("100")).toBe(1);
	});
});
