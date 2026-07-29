import { Download, FileText } from "lucide-react";
import { API_BASE } from "@/app/lib/api";
import { formatBytes } from "@/app/lib/format";
import type { ArtifactResponse } from "@/app/lib/types";

interface StepArtifactsProps {
	runId: string;
	/**
	 * Absent when the response comes from an API that predates artifacts, so
	 * the field is optional rather than an empty array.
	 */
	artifacts: ArtifactResponse[] | undefined;
}

/**
 * Download URL for an artifact.
 *
 * The API authenticates through the session cookie, so a plain link works and
 * the browser handles the download itself.
 */
export function artifactDownloadUrl(
	runId: string,
	artifact: ArtifactResponse,
): string {
	return `${API_BASE}/api/v1/runs/${runId}/steps/${artifact.step_id}/artifacts/${encodeURIComponent(artifact.name)}`;
}

/** Files a step produced, each with a download link. */
export function StepArtifacts({ runId, artifacts }: StepArtifactsProps) {
	if (!artifacts || artifacts.length === 0) return null;

	return (
		<div>
			<div className="text-xs font-semibold text-muted-foreground mb-1.5">
				Artifacts
			</div>
			<ul className="divide-y divide-border border rounded-md bg-muted/50">
				{artifacts.map((artifact) => (
					<li
						key={artifact.id}
						className="flex items-center gap-3 px-3 py-2 text-xs"
					>
						<FileText
							aria-hidden={true}
							className="h-3.5 w-3.5 shrink-0 text-muted-foreground"
						/>
						<span className="font-mono truncate text-foreground">
							{artifact.name}
						</span>
						<span className="ml-auto shrink-0 text-muted-foreground">
							{artifact.content_type}
						</span>
						<span className="shrink-0 tabular-nums text-muted-foreground">
							{formatBytes(artifact.size_bytes)}
						</span>
						<a
							href={artifactDownloadUrl(runId, artifact)}
							download={artifact.name}
							aria-label={`Download ${artifact.name}`}
							className="shrink-0 text-muted-foreground hover:text-foreground"
						>
							<Download aria-hidden={true} className="h-3.5 w-3.5" />
						</a>
					</li>
				))}
			</ul>
		</div>
	);
}
