import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import {
	AccountFormFields,
	EMPTY_ACCOUNT_FORM,
	parseAccountForm,
} from "./AccountFormFields";

const noop = () => undefined;

describe("AccountFormFields", () => {
	it("parses the raw inputs into request fields", () => {
		expect(
			parseAccountForm({
				...EMPTY_ACCOUNT_FORM,
				tags: " perso, ,team ",
				priority: "10",
				maxConcurrency: "",
				threshold: "80",
			}),
		).toEqual({
			tags: ["perso", "team"],
			priority: 10,
			max_concurrency: null,
			alert_threshold: 0.8,
		});
		expect(
			parseAccountForm({ ...EMPTY_ACCOUNT_FORM, maxConcurrency: "2" })
				.max_concurrency,
		).toBe(2);
	});

	it("asks for the identifier and a required token on create", () => {
		render(
			<AccountFormFields
				mode="create"
				idPrefix="t"
				values={EMPTY_ACCOUNT_FORM}
				onChange={noop}
				tokenLabel="OAuth token"
			/>,
		);
		expect(screen.getByLabelText("Identifier")).toBeRequired();
		expect(screen.getByLabelText("OAuth token")).toBeRequired();
	});

	it("hides the identifier and makes the token optional on edit", () => {
		render(
			<AccountFormFields
				mode="edit"
				idPrefix="t"
				values={EMPTY_ACCOUNT_FORM}
				onChange={noop}
			/>,
		);
		expect(screen.queryByLabelText("Identifier")).not.toBeInTheDocument();
		expect(screen.getByLabelText(/New token/)).not.toBeRequired();
	});
});
