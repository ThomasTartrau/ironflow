import { describe, it, expect } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { RunLabels } from "./RunLabels";

const LONG_KEY_A = `${"a".repeat(60)}-key`;
const LONG_VALUE_A = "v".repeat(60);
const LONG_KEY_B = `${"b".repeat(60)}-key`;
const LONG_VALUE_B = "w".repeat(60);

const longLabels = {
	[LONG_KEY_A]: LONG_VALUE_A,
	[LONG_KEY_B]: LONG_VALUE_B,
};

describe("RunLabels", () => {
	it("renders nothing when labels are undefined", () => {
		const { container } = render(<RunLabels />);
		expect(container).toBeEmptyDOMElement();
	});

	it("renders nothing when labels are empty", () => {
		const { container } = render(<RunLabels labels={{}} />);
		expect(container).toBeEmptyDOMElement();
	});

	it("truncates long labels inside a shrinkable badge", () => {
		render(<RunLabels labels={longLabels} />);
		for (const [key, value] of Object.entries(longLabels)) {
			const text = screen.getByText(`${key}: ${value}`);
			expect(text).toBeInTheDocument();
			expect(text).toHaveClass("truncate");
			const badge = text.parentElement;
			expect(badge).toHaveClass("max-w-full");
			expect(badge).toHaveClass("min-w-0");
		}
	});

	it("constrains the wrapper container to its parent width", () => {
		const { container } = render(<RunLabels labels={longLabels} />);
		const wrapper = container.firstElementChild;
		expect(wrapper).toHaveClass("min-w-0");
		expect(wrapper).toHaveClass("max-w-full");
	});

	it("renders a +N badge when labels exceed maxVisible", () => {
		render(<RunLabels labels={{ a: "1", b: "2", c: "3" }} />);
		expect(screen.getByText("+1")).toBeInTheDocument();
		expect(screen.queryByText("c: 3")).not.toBeInTheDocument();
	});

	it("shows the full label in a tooltip on hover", async () => {
		const user = userEvent.setup();
		render(<RunLabels labels={longLabels} />);
		const [key, value] = Object.entries(longLabels)[0];
		await user.hover(screen.getByText(`${key}: ${value}`));
		// Base UI's tooltip popup carries no role="tooltip"; target its slot instead.
		const tooltip = await waitFor(() => {
			const popup = document.querySelector('[data-slot="tooltip-content"]');
			expect(popup).not.toBeNull();
			return popup as HTMLElement;
		});
		expect(tooltip).toHaveTextContent(`${key}: ${value}`);
	});
});
