import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { CostBudgetCard, costLevel, costRatio } from "./CostBudgetCard";

describe("costRatio", () => {
	it("returns the consumed share of the cap", () => {
		expect(costRatio(0, 2)).toBe(0);
		expect(costRatio(1, 2)).toBe(0.5);
		expect(costRatio(2, 2)).toBe(1);
	});

	it("clamps above the cap", () => {
		expect(costRatio(5, 2)).toBe(1);
	});

	it("treats a zero cap as full as soon as anything is spent", () => {
		expect(costRatio(0, 0)).toBe(0);
		expect(costRatio(0.01, 0)).toBe(1);
	});
});

describe("costLevel", () => {
	it("stays normal below 80% of the cap", () => {
		expect(costLevel(1, 2)).toBe("normal");
		expect(costLevel(1.59, 2)).toBe("normal");
	});

	it("warns from 80% of the cap", () => {
		expect(costLevel(1.6, 2)).toBe("warning");
		expect(costLevel(1.99, 2)).toBe("warning");
	});

	it("flags as exceeded once the cap is reached", () => {
		expect(costLevel(2, 2)).toBe("exceeded");
		expect(costLevel(3, 2)).toBe("exceeded");
	});
});

describe("CostBudgetCard", () => {
	it("renders a plain cost stat when the run has no cap", () => {
		render(<CostBudgetCard cost={1.5} maxCost={null} />);

		expect(screen.getByText("$1.50")).toBeTruthy();
		expect(screen.queryByRole("progressbar")).toBeNull();
	});

	it("renders a plain cost stat when the cap is undefined", () => {
		render(<CostBudgetCard cost={1.5} />);
		expect(screen.queryByRole("progressbar")).toBeNull();
	});

	it("renders the cap and a progress bar when a cap is set", () => {
		render(<CostBudgetCard cost={0.5} maxCost={2} />);

		const bar = screen.getByRole("progressbar");
		expect(bar.getAttribute("aria-valuenow")).toBe("25");
		expect(screen.getByText("25% of budget")).toBeTruthy();
		expect(screen.getByText("/ $2.00")).toBeTruthy();
	});

	it("announces the budget as reached at the cap", () => {
		render(<CostBudgetCard cost={2} maxCost={2} />);

		expect(screen.getByRole("progressbar").getAttribute("aria-valuenow")).toBe(
			"100",
		);
		expect(screen.getByText("Budget reached (100%)")).toBeTruthy();
	});

	it("never reports more than 100% when the cost overshoots", () => {
		render(<CostBudgetCard cost={9} maxCost={2} />);
		expect(screen.getByRole("progressbar").getAttribute("aria-valuenow")).toBe(
			"100",
		);
	});
});
