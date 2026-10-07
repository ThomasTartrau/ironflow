import { describe, it, expect, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Provider } from "react-redux";
import { NuqsTestingAdapter, type UrlUpdateEvent } from "nuqs/adapters/testing";
import { store } from "@/app/store";
import { RunFilters } from "./RunFilters";

function renderFilters(
	searchParams: string,
	props: { priorityFilter?: boolean } = {},
) {
	const onUrlUpdate = vi.fn<(event: UrlUpdateEvent) => void>();
	render(
		<Provider store={store}>
			<NuqsTestingAdapter searchParams={searchParams} onUrlUpdate={onUrlUpdate}>
				<RunFilters {...props} />
			</NuqsTestingAdapter>
		</Provider>,
	);
	return onUrlUpdate;
}

const lastSearchParams = (onUrlUpdate: ReturnType<typeof renderFilters>) =>
	onUrlUpdate.mock.lastCall?.[0].searchParams;

describe("RunFilters priority", () => {
	it("writes a valid priority to the URL and resets the page", async () => {
		const user = userEvent.setup();
		const onUrlUpdate = renderFilters("?page=3", { priorityFilter: true });

		await user.click(screen.getByRole("button", { name: /Filters/ }));
		await user.type(screen.getByLabelText("Priority"), "-15");

		await waitFor(() => {
			expect(lastSearchParams(onUrlUpdate)?.get("priority")).toBe("-15");
			expect(lastSearchParams(onUrlUpdate)?.get("page")).toBe("1");
		});
	});

	it("keeps an out of range priority out of the URL", async () => {
		const user = userEvent.setup();
		const onUrlUpdate = renderFilters("", { priorityFilter: true });

		await user.click(screen.getByRole("button", { name: /Filters/ }));
		await user.type(screen.getByLabelText("Priority"), "250");
		// Outlast the 300ms debounce so the last URL update has been flushed.
		await new Promise((resolve) => setTimeout(resolve, 500));

		expect(screen.getByLabelText("Priority")).toHaveValue(250);
		expect(lastSearchParams(onUrlUpdate)?.has("priority")).toBe(false);
		for (const [event] of onUrlUpdate.mock.calls) {
			expect(event.searchParams.get("priority")).not.toBe("250");
		}
	});

	it("shows the active priority and removes it from its chip", async () => {
		const user = userEvent.setup();
		const onUrlUpdate = renderFilters("?priority=40", { priorityFilter: true });

		const chip = screen.getByRole("button", { name: "Remove priority filter" });
		expect(chip).toHaveTextContent("priority: 40");

		await user.click(chip);

		await waitFor(() => {
			expect(lastSearchParams(onUrlUpdate)?.has("priority")).toBe(false);
		});
	});

	it("ignores the priority where the filter is not offered", () => {
		renderFilters("?priority=40");
		expect(
			screen.queryByRole("button", { name: "Remove priority filter" }),
		).toBeNull();
	});
});
