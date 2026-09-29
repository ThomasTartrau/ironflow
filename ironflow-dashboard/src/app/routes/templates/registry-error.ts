import { ApiError } from "@/app/lib/api";

/** User-facing message for a failed `GET /templates/registry`. */
export function registryErrorMessage(error: unknown): string {
	if (error instanceof ApiError && error.status === 404) {
		return "Template registry not found. Check that IRONFLOW_REGISTRY_URL points to a repository with an index.toml at its root.";
	}
	return "Could not reach the template registry. Check your network or try again later.";
}
