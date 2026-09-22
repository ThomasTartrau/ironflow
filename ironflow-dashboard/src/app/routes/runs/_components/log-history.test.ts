import { describe, it, expect, vi, beforeEach } from "vitest";
import { fetchRunLogHistory, mergeLive, type LogEntry } from "./log-history";

let mockFetch: ReturnType<typeof vi.fn>;

beforeEach(() => {
	mockFetch = vi.fn();
	vi.stubGlobal("fetch", mockFetch);
});

function jsonResponse(body: unknown): Response {
	return new Response(JSON.stringify(body), {
		status: 200,
		headers: { "Content-Type": "application/json" },
	});
}

function apiEntry(id: string, line: string, stepId = "step-1") {
	return {
		id,
		run_id: "run-1",
		step_id: stepId,
		step_name: "build",
		stream: "stdout",
		line,
		created_at: "2026-01-01T00:00:00Z",
	};
}

function entry(
	id: string,
	line: string,
	at = "2026-01-01T00:00:00Z",
): LogEntry {
	return {
		id,
		stepId: "step-1",
		stepName: "build",
		stream: "stdout",
		line,
		at,
	};
}

describe("fetchRunLogHistory", () => {
	it("follows the cursor until next_cursor is null and concatenates pages", async () => {
		mockFetch
			.mockResolvedValueOnce(
				jsonResponse({
					data: [apiEntry("id-1", "line 1"), apiEntry("id-2", "line 2")],
					meta: { next_cursor: "id-2", has_more: true },
				}),
			)
			.mockResolvedValueOnce(
				jsonResponse({
					data: [apiEntry("id-3", "line 3")],
					meta: { next_cursor: null, has_more: false },
				}),
			);

		const lines = await fetchRunLogHistory("run-1");

		expect(lines.map((l) => l.line)).toEqual(["line 1", "line 2", "line 3"]);
		expect(mockFetch).toHaveBeenCalledTimes(2);
		const secondUrl = mockFetch.mock.calls[1][0] as string;
		expect(secondUrl).toContain("cursor=id-2");
		expect(secondUrl).toContain("limit=1000");
	});

	it("returns mapped entries for a single page (id and created_at -> at)", async () => {
		mockFetch.mockResolvedValueOnce(
			jsonResponse({
				data: [apiEntry("id-1", "only line")],
				meta: { next_cursor: null, has_more: false },
			}),
		);

		const lines = await fetchRunLogHistory("run-1");

		expect(mockFetch).toHaveBeenCalledTimes(1);
		expect(lines).toEqual([
			{
				id: "id-1",
				stepId: "step-1",
				stepName: "build",
				stream: "stdout",
				line: "only line",
				at: "2026-01-01T00:00:00Z",
			},
		]);
	});

	it("threads step_id and stream filters into the request", async () => {
		mockFetch.mockResolvedValueOnce(
			jsonResponse({ data: [], meta: { next_cursor: null, has_more: false } }),
		);

		await fetchRunLogHistory("run-1", { stepId: "step-9", stream: "stderr" });

		const url = mockFetch.mock.calls[0][0] as string;
		expect(url).toContain("step_id=step-9");
		expect(url).toContain("stream=stderr");
	});
});

describe("mergeLive", () => {
	it("drops the boundary duplicate: a live line whose id is in the history", () => {
		const history = [entry("id-1", "line 1"), entry("id-2", "line 2")];
		const live = [entry("id-2", "line 2"), entry("id-3", "line 3")];

		const merged = mergeLive(history, live);

		expect(merged.map((l) => l.id)).toEqual(["id-1", "id-2", "id-3"]);
	});

	it("keeps distinct lines and legitimate repeats with different ids", () => {
		const history = [entry("id-1", "same text")];
		// Same content, different id: a genuine repeat, must be kept.
		const live = [entry("id-2", "same text"), entry("id-3", "new text")];

		const merged = mergeLive(history, live);

		expect(merged.map((l) => l.id)).toEqual(["id-1", "id-2", "id-3"]);
	});

	it("keeps live lines without a stable id (nil / empty)", () => {
		const history = [entry("id-1", "line 1")];
		const live = [
			entry("", "legacy a"),
			entry("00000000-0000-0000-0000-000000000000", "legacy b"),
		];

		const merged = mergeLive(history, live);

		expect(merged.map((l) => l.line)).toEqual([
			"line 1",
			"legacy a",
			"legacy b",
		]);
	});
});
