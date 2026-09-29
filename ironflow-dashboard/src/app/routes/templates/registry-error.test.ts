import { describe, expect, it } from "vitest";
import { ApiError } from "@/app/lib/api";
import { registryErrorMessage } from "./registry-error";

describe("registryErrorMessage", () => {
	it("says the registry does not exist on a 404", () => {
		const message = registryErrorMessage(
			new ApiError(404, "template registry not found"),
		);
		expect(message).toMatch(/not found/i);
		expect(message).toContain("IRONFLOW_REGISTRY_URL");
	});

	it("says the registry could not be reached on a 502", () => {
		const message = registryErrorMessage(
			new ApiError(502, "template registry unreachable"),
		);
		expect(message).toMatch(/could not reach/i);
	});

	it("treats a network failure as unreachable", () => {
		expect(registryErrorMessage(new TypeError("Failed to fetch"))).toMatch(
			/could not reach/i,
		);
	});
});
