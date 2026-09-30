import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import type { ProviderAccountResponse } from "@/app/lib/types";
import { AccountCard } from "./AccountCard";

function accountFixture(
	overrides: Partial<ProviderAccountResponse> = {},
): ProviderAccountResponse {
	const now = new Date();
	return {
		id: "0192f0c1-0000-7000-8000-000000000001",
		name: "perso-max",
		display_name: "Perso Max",
		kind: "claude_subscription",
		enabled: true,
		priority: 10,
		tags: ["perso"],
		max_concurrency: null,
		alert_threshold: 0.8,
		expires_at: new Date(now.getTime() + 86_400_000).toISOString(),
		plan: "max",
		created_by: null,
		created_at: now.toISOString(),
		updated_at: now.toISOString(),
		auth_failed_at: null,
		state: "ok",
		windows: [
			{
				window: "five_hour",
				utilization: 0.42,
				resets_at: new Date(now.getTime() + 7_200_000).toISOString(),
				status: "allowed",
				model_scope: null,
				observed_at: now.toISOString(),
			},
			{
				window: "seven_day",
				utilization: 0.61,
				resets_at: null,
				status: "allowed",
				model_scope: null,
				observed_at: new Date(now.getTime() - 3 * 3_600_000).toISOString(),
			},
		],
		...overrides,
	};
}

const noop = () => undefined;

describe("AccountCard", () => {
	it("renders the state pill and one gauge per window", () => {
		render(
			<AccountCard
				account={accountFixture()}
				onUpdate={noop}
				onTest={noop}
				onDelete={noop}
			/>,
		);
		expect(screen.getByTestId("account-state")).toHaveTextContent("OK");
		expect(screen.getAllByTestId("account-gauge")).toHaveLength(2);
		expect(screen.getByText("42%")).toBeInTheDocument();
		expect(screen.getByText("61%")).toBeInTheDocument();
	});

	it("greys a stale observation", () => {
		render(
			<AccountCard
				account={accountFixture()}
				onUpdate={noop}
				onTest={noop}
				onDelete={noop}
			/>,
		);
		const stale = screen.getByText(/observed 3h ago/);
		expect(stale).toHaveAttribute("data-stale", "true");
		expect(stale.className).toContain("opacity-50");
		const fresh = screen.getByText("observed now");
		expect(fresh).toHaveAttribute("data-stale", "false");
	});

	it("shows a limited account", () => {
		render(
			<AccountCard
				account={accountFixture({ state: "limited" })}
				onUpdate={noop}
				onTest={noop}
				onDelete={noop}
			/>,
		);
		expect(screen.getByTestId("account-state")).toHaveTextContent("Limited");
	});

	it("labels the actions and explains the kind", () => {
		render(
			<AccountCard
				account={accountFixture()}
				onUpdate={noop}
				onTest={noop}
				onDelete={noop}
			/>,
		);
		expect(
			screen.getByRole("button", { name: "Edit account" }),
		).toBeInTheDocument();
		expect(
			screen.getByRole("button", { name: "Test the token now" }),
		).toBeInTheDocument();
		expect(
			screen.getByRole("button", { name: "Delete account" }),
		).toBeInTheDocument();
		expect(
			screen.getByText(/Claude subscription, Max plan/),
		).toBeInTheDocument();
		expect(screen.queryByText(/claude_subscription/)).not.toBeInTheDocument();
	});
});
