import { describe, it, expect, vi, beforeEach } from "vitest";
import { ApiError, api, customFetch } from "./api";

let mockFetch: ReturnType<typeof vi.fn>;

beforeEach(() => {
	mockFetch = vi.fn();
	vi.stubGlobal("fetch", mockFetch);
});

function jsonResponse(data: unknown, status = 200): Response {
	return new Response(JSON.stringify(data), {
		status,
		headers: { "Content-Type": "application/json" },
	});
}

describe("ApiError", () => {
	it("stores status and message", () => {
		const err = new ApiError(404, "not found");
		expect(err.status).toBe(404);
		expect(err.message).toBe("not found");
		expect(err).toBeInstanceOf(Error);
	});
});

describe("customFetch", () => {
	it("prepends /api/v1 to path and includes credentials", () => {
		mockFetch.mockResolvedValueOnce(jsonResponse({ data: "ok" }));

		return customFetch("/runs").then(() => {
			const [url, opts] = mockFetch.mock.calls[0] as [string, RequestInit];
			expect(url).toBe("/api/v1/runs");
			expect(opts.credentials).toBe("include");
		});
	});

	it("sets JSON content-type and accept headers", () => {
		mockFetch.mockResolvedValueOnce(jsonResponse({ data: "ok" }));

		return customFetch("/runs").then(() => {
			const [, opts] = mockFetch.mock.calls[0] as [string, RequestInit];
			const headers = opts.headers as Record<string, string>;
			expect(headers["Content-Type"]).toBe("application/json");
			expect(headers.Accept).toBe("application/json");
		});
	});
});

describe("api.get", () => {
	it("returns parsed data on success", () => {
		mockFetch.mockResolvedValueOnce(jsonResponse({ data: [1, 2, 3] }));

		return api.get<number[]>("/runs").then((result) => {
			expect(result.data).toEqual([1, 2, 3]);
		});
	});

	it("throws ApiError on non-ok response", () => {
		mockFetch.mockResolvedValueOnce(
			jsonResponse(
				{ error: { code: "NOT_FOUND", message: "run not found" } },
				404,
			),
		);

		return api.get("/runs/123").then(
			() => {
				throw new Error("should have thrown");
			},
			(err: ApiError) => {
				expect(err).toBeInstanceOf(ApiError);
				expect(err.status).toBe(404);
				expect(err.message).toBe("run not found");
			},
		);
	});
});

describe("api.post", () => {
	it("sends POST with JSON body", () => {
		mockFetch.mockResolvedValueOnce(jsonResponse({ data: { id: "abc" } }));

		return api.post("/runs", { workflow: "test" }).then((result) => {
			expect(result.data).toEqual({ id: "abc" });
			const [, opts] = mockFetch.mock.calls[0] as [string, RequestInit];
			expect(opts.method).toBe("POST");
			expect(opts.body).toBe(JSON.stringify({ workflow: "test" }));
		});
	});

	it("sends POST without body when body is undefined", () => {
		mockFetch.mockResolvedValueOnce(jsonResponse({ data: null }));

		return api.post("/auth/sign-out").then(() => {
			const [, opts] = mockFetch.mock.calls[0] as [string, RequestInit];
			expect(opts.method).toBe("POST");
			expect(opts.body).toBeUndefined();
		});
	});
});

describe("auto-refresh on 401", () => {
	it("retries request after successful token refresh", () => {
		// First call returns 401
		mockFetch.mockResolvedValueOnce(
			jsonResponse({ error: { message: "unauthorized" } }, 401),
		);
		// Refresh call succeeds
		mockFetch.mockResolvedValueOnce(jsonResponse({}, 204));
		// Retry call succeeds
		mockFetch.mockResolvedValueOnce(jsonResponse({ data: "refreshed" }));

		return api.get<string>("/runs").then((result) => {
			expect(result.data).toBe("refreshed");
			expect(mockFetch).toHaveBeenCalledTimes(3);
		});
	});

	it("does not refresh on auth skip paths", () => {
		mockFetch.mockResolvedValueOnce(
			jsonResponse({ error: { message: "invalid credentials" } }, 401),
		);

		return api.get("/auth/sign-in").then(
			() => {
				throw new Error("should have thrown");
			},
			(err: ApiError) => {
				expect(err.status).toBe(401);
				// Only 1 call: no refresh attempt
				expect(mockFetch).toHaveBeenCalledTimes(1);
			},
		);
	});
});
