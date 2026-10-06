import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router";
import { TriggerBadge } from "./TriggerBadge";
import type { TriggerKind } from "@/app/lib/types";

describe("TriggerBadge", () => {
	it("renders 'Cron' for a cron trigger with schedule in tooltip", () => {
		const trigger: TriggerKind = { kind: "cron", schedule: "0 8 * * *" };
		render(<TriggerBadge trigger={trigger} />);
		expect(screen.getByText("Cron")).toBeInTheDocument();
	});

	it("shows the occurrence of a scheduled cron run in the tooltip", async () => {
		const trigger: TriggerKind = {
			kind: "cron",
			schedule: "0 8 * * *",
			schedule_id: "019a3f2b-0000-7000-8000-0000000000bb",
			scheduled_for: "2026-10-06T08:00:00Z",
		};
		render(<TriggerBadge trigger={trigger} />);
		const badge = screen.getByText("Cron");

		const user = userEvent.setup();
		await user.hover(badge);
		const tooltip = await screen.findByText(
			"0 8 * * * (occurrence 2026-10-06T08:00:00Z)",
			{},
			{ timeout: 2000 },
		);
		expect(tooltip).toBeInTheDocument();
	});

	it("renders 'Manual' for a manual trigger without tooltip", () => {
		const trigger: TriggerKind = { kind: "manual" };
		render(<TriggerBadge trigger={trigger} />);
		expect(screen.getByText("Manual")).toBeInTheDocument();
	});

	it("renders 'API' for an api trigger", () => {
		const trigger: TriggerKind = { kind: "api" };
		render(<TriggerBadge trigger={trigger} />);
		expect(screen.getByText("API")).toBeInTheDocument();
	});

	it("renders 'Webhook' for a webhook trigger", () => {
		const trigger: TriggerKind = { kind: "webhook", path: "/hooks/github" };
		render(<TriggerBadge trigger={trigger} />);
		expect(screen.getByText("Webhook")).toBeInTheDocument();
	});

	it("renders 'NATS' for a nats trigger", () => {
		const trigger: TriggerKind = { kind: "nats", subject: "events.deploy" };
		render(<TriggerBadge trigger={trigger} />);
		expect(screen.getByText("NATS")).toBeInTheDocument();
	});

	it("renders 'Replay' for a replay trigger with a link to the original run", async () => {
		const originalRunId = "019a3f2b-0000-7000-8000-0000000000aa";
		const trigger: TriggerKind = {
			kind: "replay",
			original_run_id: originalRunId,
		};
		render(
			<MemoryRouter>
				<TriggerBadge trigger={trigger} />
			</MemoryRouter>,
		);
		const badge = screen.getByText("Replay");
		expect(badge).toBeInTheDocument();

		const user = userEvent.setup();
		await user.hover(badge);
		const link = await screen.findByRole(
			"link",
			{ name: originalRunId },
			{ timeout: 2000 },
		);
		expect(link).toHaveAttribute("href", `/runs/${originalRunId}`);
	});
});
