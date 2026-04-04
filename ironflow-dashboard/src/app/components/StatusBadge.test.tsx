import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { StatusBadge } from "./StatusBadge";
import type { RunStatus, StepStatus } from "@/app/lib/types";

describe("StatusBadge", () => {
	const runStatuses: RunStatus[] = [
		"pending",
		"running",
		"completed",
		"failed",
		"retrying",
		"cancelled",
		"awaiting_approval",
	];

	const stepStatuses: StepStatus[] = [
		"pending",
		"running",
		"completed",
		"failed",
		"skipped",
		"awaiting_approval",
		"rejected",
	];

	it.each(
		runStatuses,
	)("renders run status '%s' with capitalized label", (status) => {
		render(<StatusBadge status={status} />);
		const expected = status.charAt(0).toUpperCase() + status.slice(1);
		expect(screen.getByText(expected)).toBeInTheDocument();
	});

	it.each(
		stepStatuses,
	)("renders step status '%s' with capitalized label", (status) => {
		render(<StatusBadge status={status} />);
		const expected = status.charAt(0).toUpperCase() + status.slice(1);
		expect(screen.getByText(expected)).toBeInTheDocument();
	});

	it("applies amber styles for pending", () => {
		const { container } = render(<StatusBadge status="pending" />);
		const badge = container.firstElementChild;
		expect(badge?.className).toContain("bg-amber-100");
	});

	it("applies blue styles with pulse for running", () => {
		const { container } = render(<StatusBadge status="running" />);
		const badge = container.firstElementChild;
		expect(badge?.className).toContain("bg-blue-100");
		expect(badge?.className).toContain("animate-pulse");
	});

	it("applies emerald styles for completed", () => {
		const { container } = render(<StatusBadge status="completed" />);
		const badge = container.firstElementChild;
		expect(badge?.className).toContain("bg-emerald-100");
	});

	it("applies red styles for failed", () => {
		const { container } = render(<StatusBadge status="failed" />);
		const badge = container.firstElementChild;
		expect(badge?.className).toContain("bg-red-100");
	});

	it("applies gray styles for cancelled", () => {
		const { container } = render(<StatusBadge status="cancelled" />);
		const badge = container.firstElementChild;
		expect(badge?.className).toContain("bg-gray-100");
	});

	it("applies gray styles for skipped", () => {
		const { container } = render(<StatusBadge status="skipped" />);
		const badge = container.firstElementChild;
		expect(badge?.className).toContain("bg-gray-100");
	});

	it("applies rose styles for rejected", () => {
		const { container } = render(<StatusBadge status="rejected" />);
		const badge = container.firstElementChild;
		expect(badge?.className).toContain("bg-rose-100");
	});

	it("applies purple styles with pulse for awaiting_approval", () => {
		const { container } = render(<StatusBadge status="awaiting_approval" />);
		const badge = container.firstElementChild;
		expect(badge?.className).toContain("bg-purple-100");
		expect(badge?.className).toContain("animate-pulse");
	});
});
