import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { CreatedByBadge } from "./CreatedByBadge";
import type { CreatedBy } from "@/app/lib/types";

describe("CreatedByBadge", () => {
	const cases: CreatedBy[] = [
		{
			kind: "user",
			id: "019a3f2b-0000-7000-8000-000000000001",
			label: "alice",
		},
		{
			kind: "api_key",
			id: "019a3f2b-0000-7000-8000-000000000002",
			label: "ci-deploy (alice)",
		},
		{ kind: "system", id: null, label: "/hooks/github" },
	];

	it.each(cases)("renders the label for kind '$kind'", (createdBy) => {
		render(<CreatedByBadge createdBy={createdBy} />);
		expect(screen.getByText(createdBy.label)).toBeInTheDocument();
	});

	it("renders an icon for every kind", () => {
		for (const createdBy of cases) {
			const { container, unmount } = render(
				<CreatedByBadge createdBy={createdBy} />,
			);
			expect(container.querySelector("svg")).not.toBeNull();
			unmount();
		}
	});

	it("keeps a long label from breaking the layout", () => {
		render(
			<CreatedByBadge
				createdBy={{
					kind: "api_key",
					id: "019a3f2b-0000-7000-8000-000000000003",
					label: "a-very-long-api-key-name (a-very-long-username)",
				}}
			/>,
		);
		const label = screen.getByText(
			"a-very-long-api-key-name (a-very-long-username)",
		);
		expect(label.className).toContain("truncate");
	});
});
