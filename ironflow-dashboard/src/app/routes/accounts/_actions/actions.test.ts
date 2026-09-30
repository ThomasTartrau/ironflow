import { describe, it, expect, vi, beforeEach } from "vitest";

vi.mock("@/app/lib/api", () => ({
	api: {
		get: vi.fn(),
		post: vi.fn(),
		patch: vi.fn(),
		del: vi.fn(),
	},
}));

import { api } from "@/app/lib/api";
import {
	createAccount,
	deleteAccount,
	listAccountKinds,
	listAccounts,
	testAccount,
	updateAccount,
} from "./actions";

const mockApi = vi.mocked(api);

beforeEach(() => {
	vi.clearAllMocks();
});

describe("provider account actions", () => {
	it("lists accounts", async () => {
		mockApi.get.mockResolvedValueOnce({ data: [{ name: "perso-max" }] });
		const result = await listAccounts();
		expect(mockApi.get).toHaveBeenCalledWith("/provider-accounts?per_page=100");
		expect(result).toEqual([{ name: "perso-max" }]);
	});

	it("creates an account with its token", async () => {
		mockApi.post.mockResolvedValueOnce({ data: { name: "perso" } });
		const body = {
			name: "perso",
			kind: "claude_subscription",
			token: "sk-ant-oat01-x",
		};
		await createAccount(body);
		expect(mockApi.post).toHaveBeenCalledWith("/provider-accounts", body);
	});

	it("encodes the account in paths", async () => {
		mockApi.patch.mockResolvedValueOnce({ data: {} });
		await updateAccount("a/b", { enabled: false });
		expect(mockApi.patch).toHaveBeenCalledWith("/provider-accounts/a%2Fb", {
			enabled: false,
		});
	});

	it("deletes and tests", async () => {
		mockApi.del.mockResolvedValueOnce({ data: undefined });
		await deleteAccount("perso");
		expect(mockApi.del).toHaveBeenCalledWith("/provider-accounts/perso");

		mockApi.post.mockResolvedValueOnce({
			data: { result: "valid", windows: [] },
		});
		const outcome = await testAccount("perso");
		expect(mockApi.post).toHaveBeenCalledWith("/provider-accounts/perso/test");
		expect(outcome.result).toBe("valid");
	});

	it("lists kinds", async () => {
		mockApi.get.mockResolvedValueOnce({
			data: [{ id: "claude_subscription" }],
		});
		const kinds = await listAccountKinds();
		expect(mockApi.get).toHaveBeenCalledWith("/provider-accounts/kinds");
		expect(kinds[0].id).toBe("claude_subscription");
	});
});
