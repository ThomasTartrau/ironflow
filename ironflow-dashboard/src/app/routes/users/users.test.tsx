import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router";

vi.mock("@/app/lib/api", () => ({
	api: {
		get: vi.fn().mockResolvedValue({ data: [], meta: null }),
		delete: vi.fn().mockResolvedValue({}),
	},
}));

vi.mock("@/app/lib/api-toast", () => ({
	withToast: vi.fn((promise: Promise<unknown>) => promise),
}));

vi.mock("./_actions/actions", () => ({
	deleteUser: vi.fn().mockResolvedValue({}),
	updateUserRole: vi.fn().mockResolvedValue({}),
}));

vi.mock("react-router", async () => {
	const actual = await vi.importActual("react-router");
	return {
		...actual,
		useLoaderData: () => ({
			users: [
				{
					id: "self-id",
					username: "thomas.tartrau",
					email: "thomas@test.com",
					is_admin: true,
					created_at: "2026-01-01T00:00:00Z",
				},
				{
					id: "other-id",
					username: "alice",
					email: "alice@test.com",
					is_admin: false,
					created_at: "2026-01-01T00:00:00Z",
				},
			],
		}),
		useNavigate: () => vi.fn(),
		useRevalidator: () => ({ revalidate: vi.fn() }),
	};
});

vi.mock("@/app/store", () => ({
	useAppSelector: () => ({
		status: "authenticated",
		user: { user_id: "self-id", is_admin: true },
	}),
}));

import { Component } from "./index";

function renderUsers() {
	return render(
		<MemoryRouter>
			<Component />
		</MemoryRouter>,
	);
}

describe("User delete confirmation", () => {
	beforeEach(() => {
		vi.clearAllMocks();
	});

	it("requires username input when deleting own account", async () => {
		const user = userEvent.setup();
		renderUsers();

		const deleteButtons = screen.getAllByRole("button", {
			name: /delete user/i,
		});
		await user.click(deleteButtons[0]);

		const deleteConfirmBtn = screen.getByRole("button", { name: "Delete" });
		expect(deleteConfirmBtn).toBeDisabled();

		const input = screen.getByPlaceholderText("thomas.tartrau");
		await user.type(input, "thomas.tartrau");

		expect(deleteConfirmBtn).toBeEnabled();
	});

	it("does not require username input when deleting another user", async () => {
		const user = userEvent.setup();
		renderUsers();

		const deleteButtons = screen.getAllByRole("button", {
			name: /delete user/i,
		});
		await user.click(deleteButtons[1]);

		const deleteConfirmBtn = screen.getByRole("button", { name: "Delete" });
		expect(deleteConfirmBtn).toBeEnabled();
	});
});
