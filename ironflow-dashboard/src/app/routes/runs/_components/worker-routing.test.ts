import { describe, it, expect } from "vitest";
import { workerRoutingWarning } from "./worker-routing";

describe("workerRoutingWarning", () => {
	it("warns when no worker was seen recently", () => {
		expect(
			workerRoutingWarning("pending", {
				seen_workers: 0,
				eligible_workers: 0,
			}),
		).toBe("Aucun worker n'a été vu récemment");
	});

	it("warns when no worker seen can take the run", () => {
		expect(
			workerRoutingWarning("retrying", {
				seen_workers: 3,
				eligible_workers: 0,
			}),
		).toBe(
			"Aucun worker vu récemment ne peut prendre ce run (workflow ou tags manquants)",
		);
	});

	it("is null while an eligible worker was seen", () => {
		expect(
			workerRoutingWarning("pending", {
				seen_workers: 3,
				eligible_workers: 1,
			}),
		).toBeNull();
	});

	it("is null without routing information", () => {
		expect(workerRoutingWarning("pending", null)).toBeNull();
		expect(workerRoutingWarning("pending", undefined)).toBeNull();
	});

	it("is null once the run left the queue", () => {
		expect(
			workerRoutingWarning("running", {
				seen_workers: 0,
				eligible_workers: 0,
			}),
		).toBeNull();
	});
});
