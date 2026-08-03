import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { ArtifactResponse } from "@/app/lib/types";
import { StepArtifacts, artifactDownloadUrl } from "./StepArtifacts";

const RUN_ID = "019a3f2b-0000-7000-8000-0000000000ff";
const STEP_ID = "019a3f2b-0000-7000-8000-000000000001";

function artifactFixture(
	overrides: Partial<ArtifactResponse> = {},
): ArtifactResponse {
	return {
		id: "019a3f2b-0000-7000-8000-00000000000a",
		step_id: STEP_ID,
		name: "report.html",
		content_type: "text/html",
		size_bytes: 145_408,
		sha256: "0".repeat(64),
		created_at: "2026-01-01T00:00:00Z",
		...overrides,
	};
}

describe("StepArtifacts", () => {
	it("renders nothing when the step produced no artifact", () => {
		const { container } = render(
			<StepArtifacts runId={RUN_ID} artifacts={[]} />,
		);
		expect(container).toBeEmptyDOMElement();
	});

	it("renders nothing when the field is absent from the response", () => {
		const { container } = render(
			<StepArtifacts runId={RUN_ID} artifacts={undefined} />,
		);
		expect(container).toBeEmptyDOMElement();
	});

	it("lists each artifact with its name, type and human-readable size", () => {
		render(<StepArtifacts runId={RUN_ID} artifacts={[artifactFixture()]} />);

		expect(screen.getByText("report.html")).toBeInTheDocument();
		expect(screen.getByText("text/html")).toBeInTheDocument();
		expect(screen.getByText("142 KB")).toBeInTheDocument();
	});

	it("points the download link at the artifact route of its own step", () => {
		render(<StepArtifacts runId={RUN_ID} artifacts={[artifactFixture()]} />);

		const link = screen.getByLabelText("Download report.html");
		expect(link).toHaveAttribute(
			"href",
			`/api/v1/runs/${RUN_ID}/steps/${STEP_ID}/artifacts/report.html`,
		);
		expect(link).toHaveAttribute("download", "report.html");
	});

	it("renders one entry per artifact", () => {
		render(
			<StepArtifacts
				runId={RUN_ID}
				artifacts={[
					artifactFixture(),
					artifactFixture({
						id: "019a3f2b-0000-7000-8000-00000000000b",
						name: "build.log",
						content_type: "text/plain",
					}),
				]}
			/>,
		);

		expect(screen.getAllByRole("listitem")).toHaveLength(2);
	});
});

describe("artifactDownloadUrl", () => {
	it("percent-encodes the artifact name", () => {
		const url = artifactDownloadUrl(RUN_ID, artifactFixture({ name: "a.b.c" }));
		expect(url).toContain("/artifacts/a.b.c");
	});
});
