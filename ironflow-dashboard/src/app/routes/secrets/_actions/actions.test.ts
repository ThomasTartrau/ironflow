import { describe, it, expect, vi, beforeEach } from "vitest";

vi.mock("@/app/lib/api", () => ({
	api: {
		get: vi.fn(),
		post: vi.fn(),
		put: vi.fn(),
		del: vi.fn(),
	},
}));

import { api } from "@/app/lib/api";
import { rotateSecrets, getKeyVersions } from "./actions";

const mockApi = vi.mocked(api);

beforeEach(() => {
	vi.clearAllMocks();
});

describe("rotateSecrets", () => {
	it("calls POST /secrets/rotate with default body", async () => {
		const response = {
			rotated: 3,
			failed: 0,
			remaining: 0,
			to_version: 2,
			last_id: null,
		};
		mockApi.post.mockResolvedValueOnce({ data: response });

		const result = await rotateSecrets();

		expect(mockApi.post).toHaveBeenCalledWith("/secrets/rotate", {});
		expect(result).toEqual(response);
	});

	it("passes request body when provided", async () => {
		const response = {
			rotated: 5,
			failed: 1,
			remaining: 10,
			to_version: 3,
			last_id: "some-id",
		};
		mockApi.post.mockResolvedValueOnce({ data: response });

		const result = await rotateSecrets({
			to_version: 3,
			batch_size: 50,
			after_id: "prev-id",
		});

		expect(mockApi.post).toHaveBeenCalledWith("/secrets/rotate", {
			to_version: 3,
			batch_size: 50,
			after_id: "prev-id",
		});
		expect(result).toEqual(response);
	});
});

describe("getKeyVersions", () => {
	it("calls GET /secrets/key-versions", async () => {
		const response = {
			active: 2,
			configured: [1, 2],
			in_use: [1, 2],
			missing: [],
			retirable: [1],
		};
		mockApi.get.mockResolvedValueOnce({ data: response });

		const result = await getKeyVersions();

		expect(mockApi.get).toHaveBeenCalledWith("/secrets/key-versions");
		expect(result).toEqual(response);
	});
});
