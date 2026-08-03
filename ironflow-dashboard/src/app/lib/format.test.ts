import { describe, it, expect } from "vitest";
import {
	capitalize,
	formatBytes,
	formatDuration,
	formatPercent,
	formatCost,
} from "./format";

describe("formatBytes", () => {
	it("keeps raw bytes below 1 KB", () => {
		expect(formatBytes(0)).toBe("0 B");
		expect(formatBytes(1023)).toBe("1023 B");
	});

	it("switches units at each 1024 boundary", () => {
		expect(formatBytes(1024)).toBe("1.0 KB");
		expect(formatBytes(1024 * 1024)).toBe("1.0 MB");
		expect(formatBytes(1024 * 1024 * 1024)).toBe("1.0 GB");
	});

	it("drops the decimal once the value reaches 10", () => {
		expect(formatBytes(145_408)).toBe("142 KB");
	});

	it("caps at the largest known unit", () => {
		expect(formatBytes(1024 ** 5)).toBe("1024 TB");
	});

	it("returns a dash for a nonsensical size", () => {
		expect(formatBytes(-1)).toBe("-");
		expect(formatBytes(Number.NaN)).toBe("-");
	});
});

describe("capitalize", () => {
	it("capitalizes first letter", () => {
		expect(capitalize("hello")).toBe("Hello");
	});

	it("returns empty string for empty input", () => {
		expect(capitalize("")).toBe("");
	});

	it("handles single character", () => {
		expect(capitalize("a")).toBe("A");
	});

	it("preserves already capitalized string", () => {
		expect(capitalize("Hello")).toBe("Hello");
	});

	it("replaces underscores with spaces and capitalizes each word", () => {
		expect(capitalize("awaiting_approval")).toBe("Awaiting Approval");
	});

	it("handles multiple underscores", () => {
		expect(capitalize("some_long_status_name")).toBe("Some Long Status Name");
	});
});

describe("formatDuration", () => {
	it("formats milliseconds below 1s", () => {
		expect(formatDuration(500)).toBe("500ms");
	});

	it("formats zero ms", () => {
		expect(formatDuration(0)).toBe("0ms");
	});

	it("formats seconds", () => {
		expect(formatDuration(2500)).toBe("2.5s");
	});

	it("formats exact seconds", () => {
		expect(formatDuration(3000)).toBe("3.0s");
	});

	it("formats minutes and seconds", () => {
		expect(formatDuration(65000)).toBe("1m 5s");
	});

	it("formats exact minutes", () => {
		expect(formatDuration(120000)).toBe("2m 0s");
	});

	it("formats large durations", () => {
		expect(formatDuration(3661000)).toBe("61m 1s");
	});
});

describe("formatPercent", () => {
	it("formats with default 1 decimal", () => {
		expect(formatPercent(50.5)).toBe("50.5%");
	});

	it("formats with custom decimals", () => {
		expect(formatPercent(33.333, 2)).toBe("33.33%");
	});

	it("formats zero", () => {
		expect(formatPercent(0)).toBe("0.0%");
	});

	it("formats 100%", () => {
		expect(formatPercent(100)).toBe("100.0%");
	});
});

describe("formatCost", () => {
	it("formats zero cost", () => {
		expect(formatCost(0)).toBe("$0.0000");
	});

	it("formats small cost below $0.01", () => {
		expect(formatCost(0.005)).toBe("$0.0050");
	});

	it("formats cost at $0.01", () => {
		expect(formatCost(0.01)).toBe("$0.01");
	});

	it("formats normal cost", () => {
		expect(formatCost(1.5)).toBe("$1.50");
	});

	it("formats large cost", () => {
		expect(formatCost(99.99)).toBe("$99.99");
	});
});
