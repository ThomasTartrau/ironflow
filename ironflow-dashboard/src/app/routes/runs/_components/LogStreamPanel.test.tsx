import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen } from "@testing-library/react";
import { LogStreamPanel } from "./LogStreamPanel";

let mockFetch: ReturnType<typeof vi.fn>;

beforeEach(() => {
	mockFetch = vi.fn();
	vi.stubGlobal("fetch", mockFetch);
});

afterEach(() => {
	vi.unstubAllGlobals();
});

function jsonResponse(body: unknown): Response {
	return new Response(JSON.stringify(body), {
		status: 200,
		headers: { "Content-Type": "application/json" },
	});
}

function apiEntry(id: string, line: string) {
	return {
		id,
		run_id: "run-1",
		step_id: "step-1",
		step_name: "build",
		stream: "stdout",
		line,
		created_at: "2026-01-01T00:00:00Z",
	};
}

describe("LogStreamPanel (terminated run)", () => {
	it("hydrates and renders the persisted history, never 'Run is not active'", async () => {
		mockFetch.mockResolvedValueOnce(
			jsonResponse({
				data: [apiEntry("id-1", "compiling"), apiEntry("id-2", "done")],
				meta: { next_cursor: null, has_more: false },
			}),
		);

		render(<LogStreamPanel runId="run-1" isActive={false} />);

		expect(await screen.findByText("compiling")).toBeInTheDocument();
		expect(screen.getByText("done")).toBeInTheDocument();
		// The old dead-end copy must not reappear for a run that has logs.
		expect(screen.queryByText("Run is not active")).not.toBeInTheDocument();
		expect(
			screen.queryByText("No logs recorded for this run"),
		).not.toBeInTheDocument();
	});

	it("shows 'No logs recorded' (not 'Run is not active') when the run has none", async () => {
		mockFetch.mockResolvedValueOnce(
			jsonResponse({ data: [], meta: { next_cursor: null, has_more: false } }),
		);

		render(<LogStreamPanel runId="run-1" isActive={false} />);

		expect(
			await screen.findByText("No logs recorded for this run"),
		).toBeInTheDocument();
		expect(screen.queryByText("Run is not active")).not.toBeInTheDocument();
	});

	it("surfaces an error state when hydration fails, not 'No logs recorded'", async () => {
		mockFetch.mockRejectedValueOnce(new Error("network down"));

		render(<LogStreamPanel runId="run-1" isActive={false} />);

		expect(await screen.findByText("Failed to load logs")).toBeInTheDocument();
		expect(
			screen.queryByText("No logs recorded for this run"),
		).not.toBeInTheDocument();
	});
});
