import { describe, it, expect } from "vitest";
import { render, screen, within } from "@testing-library/react";
import { MemoryRouter } from "react-router";
import { AuditLogsTable } from "./AuditLogsTable";
import type { AuditLogEntry, UserResponse } from "@/app/lib/types";

const ENTRIES: AuditLogEntry[] = [
	{
		id: "aaaaaaaa-0000-0000-0000-000000000001",
		event_type: "run_created",
		payload: { workflow: "deploy" },
		run_id: "bbbbbbbb-0000-0000-0000-000000000099",
		step_id: null,
		user_id: "cccccccc-0000-0000-0000-000000000042",
		created_at: "2026-08-15T10:30:00Z",
	},
	{
		id: "dddddddd-0000-0000-0000-000000000002",
		event_type: "secrets_rotated",
		payload: { rotated: 5, failed: 0 },
		run_id: null,
		step_id: null,
		user_id: null,
		created_at: "2026-08-16T14:00:00Z",
	},
];

const USER_MAP = new Map<string, UserResponse>([
	[
		"cccccccc-0000-0000-0000-000000000042",
		{
			id: "cccccccc-0000-0000-0000-000000000042",
			email: "admin@ironflow.dev",
			username: "admin",
			is_admin: true,
			created_at: "2026-01-01T00:00:00Z",
			updated_at: "2026-01-01T00:00:00Z",
		},
	],
]);

function renderWithRouter(ui: React.ReactElement) {
	return render(<MemoryRouter>{ui}</MemoryRouter>);
}

describe("AuditLogsTable", () => {
	it("renders column headers", () => {
		renderWithRouter(<AuditLogsTable entries={ENTRIES} userMap={USER_MAP} />);
		expect(screen.getByText("Date")).toBeInTheDocument();
		expect(screen.getByText("Event")).toBeInTheDocument();
		expect(screen.getByText("Run")).toBeInTheDocument();
		expect(screen.getByText("User")).toBeInTheDocument();
		expect(screen.getByText("Payload")).toBeInTheDocument();
	});

	it("renders one row per entry with correct event type", () => {
		renderWithRouter(<AuditLogsTable entries={ENTRIES} userMap={USER_MAP} />);
		const rows = screen.getAllByRole("row");
		expect(rows).toHaveLength(3);
		expect(screen.getByText("Run Created")).toBeInTheDocument();
		expect(screen.getByText("Secrets Rotated")).toBeInTheDocument();
	});

	it("renders an empty state when entries is empty", () => {
		renderWithRouter(<AuditLogsTable entries={[]} />);
		expect(screen.getByText("No audit logs found")).toBeInTheDocument();
		expect(screen.queryAllByRole("row")).toHaveLength(0);
	});

	it("renders a View payload button per row", () => {
		renderWithRouter(<AuditLogsTable entries={ENTRIES} userMap={USER_MAP} />);
		const buttons = screen.getAllByRole("button", { name: "View payload" });
		expect(buttons).toHaveLength(2);
	});

	it("renders run ID as a clickable link when present, dash when absent", () => {
		renderWithRouter(<AuditLogsTable entries={ENTRIES} userMap={USER_MAP} />);
		const link = screen.getByRole("link", { name: "bbbbbbbb" });
		expect(link).toHaveAttribute(
			"href",
			"/runs/bbbbbbbb-0000-0000-0000-000000000099",
		);
		const rows = screen.getAllByRole("row");
		const secondRow = rows[2];
		const cells = within(secondRow).getAllByRole("cell");
		expect(cells[2].textContent).toBe("-");
	});

	it("shows username when userMap is provided", () => {
		renderWithRouter(<AuditLogsTable entries={ENTRIES} userMap={USER_MAP} />);
		expect(screen.getByText("admin")).toBeInTheDocument();
	});

	it("falls back to truncated UUID when user is not in userMap", () => {
		renderWithRouter(<AuditLogsTable entries={ENTRIES} userMap={new Map()} />);
		expect(screen.getByText("cccccccc")).toBeInTheDocument();
	});
});
