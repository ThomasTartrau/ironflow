import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { TriggerBadge } from "./TriggerBadge";
import type { TriggerKind } from "@/app/lib/types";

describe("TriggerBadge", () => {
	it("renders 'Cron' for a cron trigger with schedule in tooltip", () => {
		const trigger: TriggerKind = { kind: "cron", schedule: "0 8 * * *" };
		render(<TriggerBadge trigger={trigger} />);
		expect(screen.getByText("Cron")).toBeInTheDocument();
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
});
