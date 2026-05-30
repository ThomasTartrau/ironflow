import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { StatusBadge } from "./StatusBadge";
import { capitalize } from "@/app/lib/format";
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
		expect(screen.getByText(capitalize(status))).toBeInTheDocument();
	});

	it.each(
		stepStatuses,
	)("renders step status '%s' with capitalized label", (status) => {
		render(<StatusBadge status={status} />);
		expect(screen.getByText(capitalize(status))).toBeInTheDocument();
	});

	it("applies the pending status token for pending", () => {
		const { container } = render(<StatusBadge status="pending" />);
		const badge = container.firstElementChild;
		expect(badge?.className).toContain("var(--status-pending-bg)");
	});

	it("applies the running status token with pulse for running", () => {
		const { container } = render(<StatusBadge status="running" />);
		const badge = container.firstElementChild;
		expect(badge?.className).toContain("var(--status-running-bg)");
		expect(badge?.className).toContain("animate-pulse");
	});

	it("applies the completed status token for completed", () => {
		const { container } = render(<StatusBadge status="completed" />);
		const badge = container.firstElementChild;
		expect(badge?.className).toContain("var(--status-completed-bg)");
	});

	it("applies the failed status token for failed", () => {
		const { container } = render(<StatusBadge status="failed" />);
		const badge = container.firstElementChild;
		expect(badge?.className).toContain("var(--status-failed-bg)");
	});

	it("applies the cancelled status token for cancelled", () => {
		const { container } = render(<StatusBadge status="cancelled" />);
		const badge = container.firstElementChild;
		expect(badge?.className).toContain("var(--status-cancelled-bg)");
	});

	it("applies the cancelled status token for skipped", () => {
		const { container } = render(<StatusBadge status="skipped" />);
		const badge = container.firstElementChild;
		expect(badge?.className).toContain("var(--status-cancelled-bg)");
	});

	it("applies the rejected status token for rejected", () => {
		const { container } = render(<StatusBadge status="rejected" />);
		const badge = container.firstElementChild;
		expect(badge?.className).toContain("var(--status-rejected-bg)");
	});

	it("applies the awaiting status token with pulse for awaiting_approval", () => {
		const { container } = render(<StatusBadge status="awaiting_approval" />);
		const badge = container.firstElementChild;
		expect(badge?.className).toContain("var(--status-awaiting-bg)");
		expect(badge?.className).toContain("animate-pulse");
	});
});
